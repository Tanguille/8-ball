# GitOps digest observation pilot

Use 8-ball to estimate which digest candidates merit deeper investigation. A candidate is one item in the digest. Hermes still receives every candidate. This pilot does not install a plugin, create or edit cron jobs, change profiles, or suppress work.

## A reviewed decision spec

Add the `[specs.gitops-investigate]` table from `examples/hermes/config.toml` to your server configuration, next to an existing endpoint.

A runnable local configuration and synthetic examples are in `examples/hermes/`. The examples demonstrate the interface, not model accuracy.

## Contextual triage

Triage filters a list of items with one question. REST `/v1/triage` and MCP `decision_triage` accept optional `context` text that every item shares:

```json
{"spec":"gitops-investigate","context":"We use kopiur and CNPG; we do not use VolSync.","items":[{"id":"change-1","title":"Kopiur restore compatibility change"}]}
```

The CLI equivalent is:

```sh
8ball triage --spec gitops-investigate --context-file inventory.txt < candidates.json
```

Each model state is the JSON object `{"context": ..., "item": ...}`. Instructions stay in the reviewed question or spec. The cache identity includes the complete serialized state, so a change to the context produces a different cached decision. 8-ball stores the context with the decision and retains it under the existing database policy. Without context, the state format and truncation behavior do not change.

For contextual requests, every serialized context and item pair must fit the state limit of the endpoint. The limit is 2,000 characters by default for System One and rerank. If one pair does not fit, 8-ball rejects the whole batch before any judgment. Supply compact, current evidence. Do not silently remove details to make an assertion pass.

Responses keep `kept` and `unsure`, and report the other items in these fields:

- `rejected`: Model-judged items that did not match `keep`, with their decisions and original indices.
- `prefiltered`: Original indices that embedding similarity excluded, without a model judgment.
- `dropped`: The sum of the two groups above.

Do not interpret a prefiltered item as a confident rejection. Items with identical serialized state collapse to their first index. 8-ball preserves the JSON object key order in contextual and context-free requests, so reordered objects stay distinct model inputs. This pilot never sets `top_k`, because you must judge every candidate to measure missed candidates.

## Observe without filtering

The collector (your code that gathers candidates) supplies a UTF-8 JSON snapshot with `context` and `items`. Each item includes its source ids and URLs. Fetch the current homelab inventory first. Keep the snapshot that each run used. Source collection and exact version and id comparisons stay in ordinary code.

```sh
umask 077
export EIGHTBALL_URL=http://127.0.0.1:8080
# Export EIGHTBALL_API_KEY if the server requires it.
8ball observe --spec gitops-investigate \
  --input examples/hermes/snapshot.json \
  --observations /tmp/8ball-gitops-run.jsonl
```

The command uses the authentication and JSON handling of the CLI. `--timeout-ms` defaults to 25,000 and covers the response body. The command refuses redirects. It needs no Python runtime.

Choose a new observation path for every run. If the input is valid and the write succeeds, the command creates a new file with mode 0600. It never overwrites an existing file. The file contains one JSONL record: `observed_at_unix_ms` (Unix milliseconds), spec, endpoint override, original snapshot, duration, complete response, and any error.

If the input is missing, unreadable, or invalid, the command creates no observation file. The error appears in the stdout payload as `context.input_error`. A write failure appears as `context.observation_error` and can leave a partial file. Observation files contain raw input text. Keep these files private. Apply your own retention policy.

The final stdout line contains `wakeAgent: true`. If the input is valid, `context.snapshot` contains the entire original snapshot. Model rejections, `unsure` results, HTTP failures, and observation write failures never remove candidates. An HTTP failure appears as `context.decision_error`.

If `context.input_error` is present, `context.snapshot` is absent. The same holds for JSON that parsed but is not valid. In this case, Hermes must fetch valid evidence and must not conclude that there were no changes. The model verdicts stay in the observation file, outside the evidence that Hermes receives, to reduce evaluation bias.

Hermes supports pre-check scripts through its [native cron `script` interface](https://hermes-agent.nousresearch.com/docs/user-guide/features/cron/#skipping-the-agent-entirely-wakeagent). A future approved installation can use a wrapper that collects a fresh snapshot, chooses a unique output path, and then invokes `8ball observe`. The two-line `examples/hermes/gitops-observe.sh` launcher executes the CLI. It contains no HTTP or decision logic. Hermes accepts a script file as the `script` value, not a shell command. Nothing here modifies a running Hermes instance.

## Replay a separate labeled holdout

A holdout is a labeled set that calibration never sees. Prepare JSONL with a unique id, the original item, its original context, and an independently reviewed `expected_keep` boolean:

```json
{"id":"restore-change","item":{"title":"Kopiur restore compatibility change"},"context":"We use kopiur.","expected_keep":true}
{"id":"unused-component","item":{"title":"VolSync maintenance release"},"context":"We use kopiur; VolSync is not installed.","expected_keep":false}
```

The label records whether investigation was worthwhile, not whether Hermes happened to investigate. Review a sample of rejected and `unsure` items as well as kept items. Split by source, event, or time so that duplicate or near-duplicate items do not leak between calibration and evaluation. Never fit bands (the calibrated verdict thresholds) on the holdout. Keep the endpoint, spec, and calibration fixed during a comparison.

```sh
umask 077
8ball doctor > /tmp/8ball-models-before.json
8ball evaluate --spec gitops-investigate --endpoint d3 \
  < examples/hermes/holdout.jsonl > /tmp/8ball-d3-evaluation.json
# Repeat with another configured endpoint against the same reviewed holdout.
```

`evaluate` makes sure that the complete dataset is valid before it sends requests. It replays one item per request through `/v1/triage` and writes one JSON report with per-case observations. It never sends expected labels to the server and never calls label or calibrate. Normal triage persistence still saves unlabeled decisions and can reuse cached answers.

Save the configuration and the `doctor` output with the experiment to identify model, spec, and calibration changes. The report does not repeat raw fixture text. Keep the original dataset beside the report.

`evaluate` has these flags:

- `--keep`: Has the same meaning as in triage (default `yes`, choice label, or minimum score index).
- `--endpoint`: Overrides the spec endpoint.
- `--timeout-ms`: Limits each HTTP request, including its response body. The default is 25,000.

Exit 0 means that all cases produced valid responses. It does not mean that the model is accurate. Exit 4 with a JSON report means that one or more cases failed. An invalid dataset exits 4 before inference, with a stderr diagnostic.

Read these report fields:

- `missed_useful`: Useful cases that were confidently rejected. This is the primary failure to inspect.
- `unnecessary_keeps`: Unhelpful cases that were confidently kept. `unsure` and error fallbacks are separate.
- `coverage`: Decided cases divided by all cases.
- `decision_accuracy`: Accuracy that excludes errors and `unsure` verdicts.
- `skip_precision`: The fraction of proposed skips that were unhelpful.
- `useful_recall`: Useful cases that were kept, including `unsure` and error fallbacks. High recall alone can mean that the service failed on everything. Always inspect errors and coverage.
- `errors`: Transport failures and responses that the evaluator cannot interpret. These cases always fail open (`would_keep: true`). They never count as skips or misses and add no latency samples. Their per-case error messages explain the failure. Recall does not establish accuracy on these cases.
- `would_skip_fraction`: Potential downstream items avoided. It does not measure token, money, or latency savings.
- `cached_latency` and `uncached_latency`: Separate nearest-rank p50 and p95 end-to-end request timings for successful cases. Uncached timings include server, probe, queue, and storage overhead, not only inference. Errors keep per-case durations but do not enter either latency distribution.

Ratios with zero denominators are `null`.

Before you propose enforcement, compare these results with measured Hermes calls, latency, and token usage. If you enable filtering later, `unsure` and error outcomes must keep the normal workflow. Incomplete or stale source evidence must not count as a negative decision. This pilot introduces no automatic promotion threshold.
