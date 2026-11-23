# GitOps digest observation pilot

A candidate is one item in the GitOps digest. Use this pilot to estimate which candidates are worth a closer look. Hermes receives every candidate while you collect 8-ball's decisions. Any plugin, scheduled job, or profile changes are separate from the pilot.

## A reviewed decision spec

A spec is a question saved in the server configuration. Add `[specs.gitops-investigate]` from `examples/hermes/config.toml` to your server configuration, next to an existing endpoint.

The `examples/hermes/` directory contains a local configuration you can run and sample inputs you can try. Use a separate test set to measure model accuracy.

## Contextual triage

Triage filters a list of items with one question. REST `/v1/triage` and MCP `decision_triage` accept optional `context` text that applies to every item.

```json
{"spec":"gitops-investigate","context":"We use kopiur and CNPG; we do not use VolSync.","items":[{"id":"change-1","title":"Kopiur restore compatibility change"}]}
```

To pass the same information from the command line, put the context in `inventory.txt` and the items in `candidates.json`.

```sh
8ball triage --spec gitops-investigate --context-file inventory.txt < candidates.json
```

The state is the text that the model judges. With shared context, each state is a JSON object, `{"context": ..., "item": ...}`. Keep the instructions in the reviewed question or spec.

8-ball uses the full JSON text when it looks up a saved decision. If the context changes, 8-ball treats it as a different state. It saves the context with the decision and uses the same retention rules. Without context, 8-ball uses its usual state format and length rules.

Every JSON pair of context and item must fit the model server's `max_state_chars` limit. See the [configuration reference](configuration.md). If any pair is too long, 8-ball rejects the whole batch before it judges any item. Keep the evidence short and current. Do not silently remove details to make an assertion pass.

The response includes `kept` and `unsure`. It reports the other items in these fields.

- `rejected` lists the items that the model judged and that did not match `keep`. It includes their decisions and original positions.
- `prefiltered` lists the original positions of items that the similarity filter excluded. The model did not judge these items.
- `dropped` counts both groups.

Each request accepts up to 64 items. `8ball triage` splits a larger array into groups of 64. If an item fails, it comes back in `unsure` with an `error` field.

If two items have identical state text, they share a decision and each keeps its own position. It preserves JSON object key order with or without context. Changing the key order makes a distinct model input. Leave `top_k` unset in the pilot so that the model judges every distinct item. You need those judgments to measure missed candidates.

## Observe without filtering

Fetch the current inventory before each run. Use your collector, the code that gathers candidates, to save a UTF-8 JSON snapshot with `context` and `items`. Include source ids and URLs for each item. Keep the snapshot that each run used. Use ordinary code to collect sources and compare exact versions and ids.

```sh
umask 077
export EIGHTBALL_URL=http://127.0.0.1:8080
# Export EIGHTBALL_API_KEY if the server requires it.
8ball observe --spec gitops-investigate \
  --input examples/hermes/snapshot.json \
  --observations /tmp/8ball-gitops-run.jsonl
```

Choose a new observation path for every run. If the input is valid and the write succeeds, the command creates a file with mode 0600. It never overwrites an existing file. It writes one JSONL record with the spec, endpoint override, original snapshot, duration, full response, and any error. The `observed_at_unix_ms` field gives the time in Unix milliseconds.

Keep observation files private and set your own retention policy. These files contain the full input text.

The last line of standard output contains `wakeAgent: true`. If the input is valid, `context.snapshot` contains the entire original snapshot. Hermes gets every candidate even if the model rejects an item, returns `unsure`, or the request or file write fails. Hermes receives the evidence without the model's verdicts so that it can review the evidence independently.

If the input is missing, unreadable, or invalid, the command leaves out `context.snapshot` and creates no observation file. It prints `context.input_error`. This includes JSON that parses but fails the input rules. If you see that error, fetch valid evidence before you draw conclusions about what changed.

A write failure appears as `context.observation_error` and can leave a partial file. An HTTP failure appears as `context.decision_error`.

After Hermes investigates, use the `result` in the observation file to find decision ids and `report` flags. Label each candidate with `report: true` using `decision_label` or `8ball label`. Use `yes` if the investigation was worthwhile and `no` otherwise.

If you later approve a scheduled run, Hermes can run a script before it wakes the agent. Use its [cron `script` interface](https://hermes-agent.nousresearch.com/docs/user-guide/features/cron/#skipping-the-agent-entirely-wakeagent) and pass a script file. The script can collect a fresh snapshot, choose a new path, and run `8ball observe`. The launcher `examples/hermes/gitops-observe.sh` only starts the CLI.

## Replay a separate labeled holdout

A holdout is a labeled test set that you leave out of calibration. For each JSONL record, include a unique id, the original item, its original context, and an independently reviewed `expected_keep` boolean.

```json
{"id":"restore-change","item":{"title":"Kopiur restore compatibility change"},"context":"We use kopiur.","expected_keep":true}
{"id":"unused-component","item":{"title":"VolSync maintenance release"},"context":"We use kopiur; VolSync is not installed.","expected_keep":false}
```

Label each case based on whether it was worth investigating. Review a sample of kept, rejected, and `unsure` items. Split cases by source, event, or time. Keep duplicates and near-duplicates together so that calibration and evaluation use separate examples.

Keep the holdout out of calibration. During a comparison, keep the endpoint, spec, and calibration fixed. Set `recalibrate_after = 0` in `[learning]` to stop new labels from changing the decision thresholds.

```sh
umask 077
8ball doctor > /tmp/8ball-models-before.json
8ball evaluate --spec gitops-investigate --endpoint d3 \
  < examples/hermes/holdout.jsonl > /tmp/8ball-d3-evaluation.json
# Repeat with another configured endpoint against the same reviewed holdout.
```

`evaluate` checks the entire dataset before it sends requests. It sends one item per request to `/v1/triage` and writes one JSON report with the result for each case. Expected labels stay on the client. The command makes no calls to label or calibrate. It sends `"learn": false` to turn off remembered labels and exploration. Triage still saves unlabeled decisions and can reuse saved answers.

Save the configuration and `doctor` output with the experiment. These show which model, spec, and calibration you used. Keep the original dataset beside the report. The raw input text stays in that dataset.

[Commands](cli.md) lists the `evaluate` flags.

Exit code 0 means that all cases returned valid responses. Use the report to measure accuracy. Code 4 with a JSON report means that at least one case failed. If the dataset is invalid, the command prints an error to standard error and exits with code 4 before it calls the model.

## Read the report

The p50 and p95 values give the times within which 50% and 95% of requests finish.

- `missed_useful` counts useful cases that the model confidently rejected. Look at these first.
- `unnecessary_keeps` counts unhelpful cases that the model confidently kept. This count excludes `unsure` results and errors.
- `coverage` is the share of all cases that got a clear answer.
- `decision_accuracy` is the share of clear answers that were correct. It excludes errors and `unsure` results.
- `skip_precision` is the share of proposed skips that were unhelpful.
- `useful_recall` is the share of useful cases that the workflow kept. It includes `unsure` results and errors. Even a server that fails on every case can score high here. Read this alongside coverage and errors.
- `errors` counts request failures and responses that the command cannot read. These cases always get `would_keep: true`. Errors count toward neither skips nor misses. They add no latency samples. Read the error message for each failed case.
- `would_skip_fraction` is the share of items that a later step can skip. To measure token, cost, or time savings, use separate measurements.
- `cached_latency` and `uncached_latency` give p50 and p95 request times for successful cases, using the nearest-rank method. Uncached times include model calls, server work, model checks, queueing, and storage. Errors retain their own durations outside these totals.

Ratios with zero denominators are `null`.

Before you propose filtering, compare these results with measured Hermes calls, response times, and token use. If you enable filtering later, keep the normal workflow for `unsure` results and errors. If source evidence is missing or stale, fetch current evidence before using it to reject an item. The pilot sets no automatic cutoff for turning on filtering.
