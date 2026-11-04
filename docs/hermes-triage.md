# GitOps digest observation pilot

Use 8-ball to estimate which digest candidates merit deeper investigation. Hermes still receives every candidate. This pilot does not install a plugin, create or edit cron jobs, change profiles, or suppress work.

## A reviewed decision spec

Add this spec to your server configuration alongside an existing endpoint:

```toml
[specs.gitops-investigate]
type = "noul"
instructions = "Does this candidate warrant investigating for this homelab? The state is JSON data with context (current inventory and recent recommendations) and item (the candidate). Investigate changes plausibly affecting a component we use, including migrations, breakage, or meaningful operational improvements. Routine version bumps and changes limited to components we do not use do not warrant investigation. Judge only the supplied evidence."
```

A runnable local configuration and synthetic examples are in `examples/hermes/`. The examples demonstrate the interface, not model accuracy.

## Contextual triage

REST `/v1/triage` and MCP `decision_triage` accept optional `context` text shared by every item:

```json
{"spec":"gitops-investigate","context":"We use kopiur and CNPG; we do not use VolSync.","items":[{"id":"change-1","title":"Kopiur restore compatibility change"}]}
```

Each model state is the JSON object `{"context": ..., "item": ...}`. Instructions stay in the reviewed question/spec. Cache identity includes the complete serialized state, so changing context produces a different cached decision. Context is stored with the decision and retained under the existing database policy. Without context, the existing state format and truncation behavior remain unchanged.

For contextual requests, **every serialized context/item pair must fit the endpoint's state limit** (2,000 characters by default for System One and rerank). The whole batch is rejected before any judgments if one pair would be truncated. Supply compact, current evidence; do not silently remove details to make an assertion pass.

CLI equivalent:

```sh
8ball triage --spec gitops-investigate --context-file inventory.txt < candidates.json
```

Responses retain `kept`, `unsure`, and `dropped`, and add:

- `rejected`: model-judged items that did not match `keep`, including their decisions and original indices.
- `prefiltered`: original indices excluded by embedding similarity, **without a model judgment**.

`dropped` is the sum of these two groups. Identical input items still collapse to their first index. Do not interpret a prefiltered item as a confident rejection. This pilot never sets `top_k`; measuring missed candidates requires judging every candidate.

## Observe without filtering

The collector supplies a UTF-8 JSON snapshot with `context` and `items`, including source ids/URLs inside each item. Fetch the current homelab inventory first and preserve the snapshot used for each run. Source collection and exact version/id comparisons remain ordinary code.

```sh
umask 077
export EIGHTBALL_URL=http://127.0.0.1:8080
# Export EIGHTBALL_API_KEY if the server requires it.
python3 examples/hermes/gitops_observe.py \
  --input examples/hermes/snapshot.json \
  --observations /tmp/8ball-gitops-run.jsonl
```

Choose a new observation path for every run. Each file is created exclusively with mode 0600 and contains one JSONL record: timestamp, spec, endpoint override, original snapshot, duration, complete response, and any error. An existing file is never overwritten. Observation files contain raw input text; keep them private and apply your own retention policy.

The final stdout line always contains `wakeAgent: true` with the **entire original snapshot**. Model rejections, uncertain results, HTTP failures, and observation-write failures never remove candidates. A missing or invalid input wakes Hermes with an explicit input error; it must fetch evidence rather than conclude there were no changes. The model verdicts stay in the observation file, outside Hermes' injected evidence, to reduce evaluation bias.

Hermes supports pre-check scripts through its [native cron `script` interface](https://hermes-agent.nousresearch.com/docs/user-guide/features/cron/#skipping-the-agent-entirely-wakeagent). A future approved installation can have a wrapper collect a fresh snapshot, choose a unique output path, then invoke this example. Hermes accepts a script file, not a shell command as the `script` value. Nothing in this PR modifies the running Hermes instance.

## Replay a separate labelled holdout

Prepare JSONL with a unique id, the original item, its original context, and an independently reviewed `expected_keep` boolean:

```json
{"id":"restore-change","item":{"title":"Kopiur restore compatibility change"},"context":"We use kopiur.","expected_keep":true}
{"id":"unused-component","item":{"title":"VolSync maintenance release"},"context":"We use kopiur; VolSync is not installed.","expected_keep":false}
```

The label answers **whether investigation was worthwhile**, not whether Hermes happened to investigate. Review a sample of rejected and uncertain items as well as retained ones. Split by source/event/time so duplicate or near-duplicate items do not leak between calibration and evaluation. Never fit bands on the holdout, and keep the endpoint, spec, and calibration fixed during a comparison.

```sh
umask 077
8ball doctor > /tmp/8ball-models-before.json
8ball evaluate --spec gitops-investigate --endpoint d3 \
  < examples/hermes/holdout.jsonl > /tmp/8ball-d3-evaluation.json
# Repeat with another configured endpoint against the same reviewed holdout.
```

`evaluate` validates the complete dataset before requests, replays one item per request through `/v1/triage`, and emits one JSON report with per-case observations. It never sends expected labels to the server or calls label/calibrate. Normal triage persistence still saves unlabelled decisions and may reuse cached answers. Save the config and `doctor` output with the experiment to identify model/spec/calibration changes. Raw fixture text is not repeated in the report; keep the original dataset beside it.

Options: `--keep` has the same meaning as triage (default `yes`, choice label, or minimum score index); `--endpoint` overrides the spec endpoint; `--timeout-ms` bounds each HTTP request including its response body (default 25,000). Exit 0 means all cases produced valid responses, **not that the model is accurate**. Exit 4 with a JSON report means one or more cases failed; invalid datasets exit 4 before inference, with a stderr diagnostic.

Report interpretation:

- `missed_useful`: useful cases confidently rejected — the primary failure to inspect.
- `unnecessary_keeps`: unhelpful cases confidently retained; uncertain/error fallbacks are separate.
- `coverage`: decided cases divided by all cases; `decision_accuracy` excludes errors and abstentions.
- `skip_precision`: fraction of proposed skips that were genuinely unhelpful.
- `useful_recall`: useful cases retained, including unsure/error fallbacks. High recall alone can mean the service failed on everything; always inspect errors and coverage.
- `would_skip_fraction`: potential downstream items avoided, **not measured token, money, or latency savings**.
- `cached_latency` and `uncached_latency`: separate nearest-rank p50/p95 end-to-end request timings for successful cases. Uncached timings include server/probe/queue/storage overhead, not just inference. Errors retain per-case durations but do not enter either latency distribution. Ratios with zero denominators are `null`.

Compare these results with measured Hermes calls, latency, and token usage before proposing enforcement. If filtering is later enabled, unsure/error outcomes must keep the normal workflow, and incomplete/stale source evidence must not be treated as a negative decision. No automatic promotion threshold is introduced here.

## Verify locally

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo nextest run
python3 -B -m unittest discover -s examples/hermes -p 'test_*.py'
```
