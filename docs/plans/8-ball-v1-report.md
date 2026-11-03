# 8-ball v1 local execution report

Run started 2026-11-03. Scope: Chunks 1, 2, 3, 4, 6; local only.
Branch: `feat/v1`. Worktree: `/home/tanguille/Documents/PriveProjecten/8-ball`.

## Pre-work evidence

```text
$ pwd
/home/tanguille/Documents/PriveProjecten/8-ball
$ git branch --show-current
main
$ git status --short
?? docs/
$ git log --oneline -5
fatal: your current branch 'main' does not have any commits yet
$ git remote -v
(no output)
$ git pull
error: cannot open '.git/FETCH_HEAD': Read-only file system
$ git pull  # with sandbox escalation
There is no tracking information for the current branch.
$ git commit -m "chore: add v1 plan"
[main (root-commit) ddb5240] chore: add v1 plan
$ git switch -c feat/v1
Switched to a new branch 'feat/v1'
$ cargo nextest --version
cargo-nextest 0.9.150 (d225b1daf 2026-10-26)
$ docker info --format '{{.ServerVersion}}'
29.9.2
```

Git metadata is writable through sandbox escalation. No remote exists, so pulling is inapplicable; no remote was added.

## Chunk 1

Step 1: scaffold completed. Dependencies were added using every command listed in the plan. The installed stable toolchain is used; no tools were installed.

## Deferred items

- Chunks 5 and 7: explicitly excluded from this local run.

## Final outcome

Not yet proven; implementation in progress.

### Step 1 checks

```text
$ cargo build
   Compiling axum v0.8.9
   Compiling rustls-webpki v0.103.16
   Compiling tokio-rustls v0.26.6
   Compiling rustls-platform-verifier v0.7.1
   Compiling hyper-rustls v0.27.10
   Compiling reqwest v0.13.5
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 24.68s
```

```text
$ cargo clippy --all-targets -- -D warnings
   Compiling rustls v0.23.46
    Checking rustls-webpki v0.103.16
    Checking tokio-rustls v0.26.6
    Checking rustls-platform-verifier v0.7.1
    Checking hyper-rustls v0.27.10
    Checking reqwest v0.13.5
    Checking eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 15.99s
```

```text
$ cargo nextest run
    Finished `test` profile [unoptimized + debuginfo] target(s) in 8.72s
────────────
 Nextest run ID 79233477-c68d-4aed-81fe-f0e66ad1afbd with nextest profile: default
    Starting 0 tests across 2 binaries
────────────
     Summary [   0.000s] 0 tests run: 0 passed, 0 skipped
error: no tests to run
(hint: use `--no-tests` to customize)
```

```text
$ cargo nextest run --no-tests pass
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.05s
────────────
 Nextest run ID 7af5fd75-e5f2-4dd1-b4fe-f639af0c93bb with nextest profile: default
    Starting 0 tests across 2 binaries
────────────
     Summary [   0.000s] 0 tests run: 0 passed, 0 skipped
```

`cargo fmt --all` passed with no output. Scaffold-only deviation: bare nextest exits 4 on zero tests; `--no-tests pass` verifies the empty scaffold. Subsequent steps use the prescribed bare command. No behavior exists to test yet.

### Step 1b: local stack setup (in progress)

The compose and shell files were copied from the plan. `shellcheck dev/models.sh` and `yayamlls validate dev/compose.yaml` both passed (no output).

Initial `dev/models.sh up` exited 0 and all health endpoints returned `{"status":"ok"}`, but the sanity calls exposed empty router instances:

```text
$ curl localhost:9931/v1/systemone ...
{"error":{"code":400,"message":"model name is missing from the request","type":"invalid_request_error"}}
$ curl localhost:9931/v1/models
{"data":[],"object":"list"}
$ curl localhost:9931/props
{"role":"router","max_instances":4,"models_autoload":true,"model_alias":"llama-server","model_path":"none","default_generation_settings":{"params":null,"n_ctx":0},"ui_settings":{},"build_info":"b12033-785322d17","cors_proxy_enabled":false}
$ curl localhost:9934/v1/chat/completions ...
{"error":{"code":400,"message":"model 'qwen3.5-0.8b' not found","type":"invalid_request_error"}}
$ curl localhost:9933/v1/rerank ...
{"error":{"code":400,"message":"model name is missing from the request","type":"invalid_request_error"}}
$ curl localhost:9932/v1/embeddings ...
{"error":{"code":400,"message":"model 'embeddinggemma-2' not found","type":"invalid_request_error"}}
```

An assert-based self-check against `/v1/models` failed before the fix:

```text
9931 {'data': [], 'object': 'list'}
Traceback (most recent call last):
  File "<stdin>", line 5, in <module>
AssertionError: port 9931 is healthy but has no model
```

Read the pinned image's `--help` and upstream `tools/server/server.cpp` and `common/arg.cpp` at `785322d17`. Root cause: `server_role_detect` tests `params.model.path`, `hf_repo` and `docker_repo`, omitting `url`; URL resolution occurs later for instances. `common_models_handler_apply` uses explicit `-m` as the URL download destination. Minimal correction: added `-m /cache/<filename>` alongside all four unchanged pinned URLs. This is a launcher correction; no model, assertion or application behavior was changed. Restart initiated downloads into the authorized cache.

### Step 1b green evidence

```text
$ dev/models.sh status
NAME           IMAGE                                                                                                              COMMAND                  SERVICE   CREATED              STATUS                        PORTS
dev-chat-1     ghcr.io/ggml-org/llama.cpp:server-vulkan@sha256:421137599beb2135a9e1d664fa8991a3f4f5f953440a9d7622c7a61b63185fe8   "/app/llama-server -…"   chat      About a minute ago   Up About a minute (healthy)   127.0.0.1:9934->9934/tcp
dev-d3-1       ghcr.io/ggml-org/llama.cpp:server-vulkan@sha256:421137599beb2135a9e1d664fa8991a3f4f5f953440a9d7622c7a61b63185fe8   "/app/llama-server -…"   d3        About a minute ago   Up About a minute (healthy)   127.0.0.1:9931->9931/tcp
dev-embed-1    ghcr.io/ggml-org/llama.cpp:server-vulkan@sha256:421137599beb2135a9e1d664fa8991a3f4f5f953440a9d7622c7a61b63185fe8   "/app/llama-server -…"   embed     About a minute ago   Up About a minute (healthy)   127.0.0.1:9932->9932/tcp
dev-rerank-1   ghcr.io/ggml-org/llama.cpp:server-vulkan@sha256:421137599beb2135a9e1d664fa8991a3f4f5f953440a9d7622c7a61b63185fe8   "/app/llama-server -…"   rerank    About a minute ago   Up About a minute (healthy)   127.0.0.1:9933->9933/tcp
9931 {"status":"ok"}
9932 {"status":"ok"}
9933 {"status":"ok"}
9934 {"status":"ok"}
```

Sanity curl systemone:
```json
{
  "model": "/cache/vllm-d3-nano-Q8_0.gguf",
  "answers": {
    "q": {
      "type": "noul",
      "noul": 0.9977173076246607
    }
  },
  "usage": {
    "input_tokens": 97,
    "output_tokens": 0
  }
}
```

Sanity curl models:
```json
{
  "input_modalities": [
    "text"
  ],
  "output_modalities": [
    "decisions"
  ]
}
```

Sanity curl props:
```json
{
  "total_slots": 2,
  "model_path": "/cache/vllm-d3-nano-Q8_0.gguf",
  "build_info": "b12033-785322d17"
}
```

Sanity curl chat:
```json
[
  {
    "id": 9175,
    "token": "Yes",
    "bytes": [
      89,
      101,
      115
    ],
    "logprob": -0.0857168659567833
  },
  {
    "id": 9405,
    "token": "yes",
    "bytes": [
      121,
      101,
      115
    ],
    "logprob": -3.13651704788208
  },
  {
    "id": 2665,
    "token": "No",
    "bytes": [
      78,
      111
    ],
    "logprob": -3.393833637237549
  },
  {
    "id": 248068,
    "token": "<think>",
    "bytes": [
      60,
      116,
      104,
      105,
      110,
      107,
      62
    ],
    "logprob": -6.307493686676025
  },
  {
    "id": 2083,
    "token": "no",
    "bytes": [
      110,
      111
    ],
    "logprob": -6.84080171585083
  }
]
```

Sanity curl rerank:
```json
{
  "model": "bge-reranker-v2-m3",
  "object": "list",
  "usage": {
    "prompt_tokens": 19,
    "total_tokens": 19
  },
  "results": [
    {
      "index": 0,
      "relevance_score": 2.2409818172454834
    },
    {
      "index": 1,
      "relevance_score": -10.912446975708008
    }
  ]
}
```

Sanity embedding curl: `768` dimensions. Loaded-model self-check passed on ports 9931–9934. Reranker returns logits (2.2409818172454834, -10.912446975708008); Chunk 2 must use sigmoid. `shellcheck`, `yayamlls validate`, `cargo fmt --all --check`, and `cargo clippy --all-targets -- -D warnings` passed. Nextest empty-scaffold check: 0 tests, 0 failures (`--no-tests pass`).

### Step 2: questions

```text
$ cargo nextest run --test question (red)
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
error[E0432]: unresolved imports `eightball::question::Kind`, `eightball::question::Question`, `eightball::question::validate_all`
 --> tests/question.rs:1:27
  |
1 | use eightball::question::{Kind, Question, validate_all};
  |                           ^^^^  ^^^^^^^^  ^^^^^^^^^^^^ no `validate_all` in `question`
  |                           |     |
  |                           |     no `Question` in `question`
  |                           no `Kind` in `question`

For more information about this error, try `rustc --explain E0432`.
error: could not compile `eightball` (test "question") due to 1 previous error
error: command `/home/tanguille/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo '--color=auto' test --no-run '--message-format=json-render-diagnostics' '--test=question'` exited with code 101
```

```text
$ cargo nextest run --test question (green)
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.56s
────────────
 Nextest run ID c168cbfc-aea9-48c5-a9a4-e18b5c586a11 with nextest profile: default
    Starting 4 tests across 1 binary
        PASS [   0.004s] (1/4) eightball::question score_labels
        PASS [   0.004s] (2/4) eightball::question noul_without_criteria_parses
        PASS [   0.004s] (3/4) eightball::question choice_roundtrip_keeps_order
        PASS [   0.004s] (4/4) eightball::question validate_rejects
────────────
     Summary [   0.005s] 4 tests run: 4 passed, 0 skipped
```

```text
$ cargo clippy --all-targets -- -D warnings
    Checking eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.31s
```

```text
$ cargo nextest run
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.48s
────────────
 Nextest run ID 8931f442-fdc1-464c-a6f7-0ccfbb2d00ac with nextest profile: default
    Starting 4 tests across 3 binaries
        PASS [   0.003s] (1/4) eightball::question noul_without_criteria_parses
        PASS [   0.003s] (2/4) eightball::question validate_rejects
        PASS [   0.003s] (3/4) eightball::question choice_roundtrip_keeps_order
        PASS [   0.003s] (4/4) eightball::question score_labels
────────────
     Summary [   0.003s] 4 tests run: 4 passed, 0 skipped
```

`cargo fmt --all` passed. Clippy requested `Eq` alongside `PartialEq`; added the derives without changing behavior. All four specified question tests pass, including the six validation rejection cases.

The Aikido plugin-requested external scan was rejected by automatic approval review: it would transmit repository source files outside the authorized local-only scope. No scan was performed; no external scan approval is needed to continue the plan's local checks.

### Step 3: config

```text
$ cargo nextest run --test config (red)
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
error[E0432]: unresolved imports `eightball::config::Config`, `eightball::config::ConfigError`
 --> tests/config.rs:2:25
  |
2 | use eightball::config::{Config, ConfigError};
  |                         ^^^^^^  ^^^^^^^^^^^ no `ConfigError` in `config`
  |                         |
  |                         no `Config` in `config`

For more information about this error, try `rustc --explain E0432`.
error: could not compile `eightball` (test "config") due to 1 previous error
error: command `/home/tanguille/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo '--color=auto' test --no-run '--message-format=json-render-diagnostics' '--test=config'` exited with code 101
```

```text
$ cargo nextest run --test config (green)
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.68s
────────────
 Nextest run ID cb33f86b-6834-443f-b557-7d50aac944a3 with nextest profile: default
    Starting 8 tests across 1 binary
        PASS [   0.004s] (1/8) eightball::config unknown_key_rejected
        PASS [   0.004s] (2/8) eightball::config minimal_ok
        PASS [   0.004s] (3/8) eightball::config spec_parses
        PASS [   0.004s] (4/8) eightball::config retention_default
        PASS [   0.004s] (5/8) eightball::config default_must_exist
        PASS [   0.004s] (6/8) eightball::config logprob_needs_model
        PASS [   0.004s] (7/8) eightball::config zero_concurrency
        PASS [   0.004s] (8/8) eightball::config bad_spec_name
────────────
     Summary [   0.004s] 8 tests run: 8 passed, 0 skipped
```

```text
$ cargo clippy --all-targets -- -D warnings
    Checking eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.47s
```

```text
$ cargo nextest run
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.49s
────────────
 Nextest run ID 28597f78-dc65-449f-8f56-3af128c59f71 with nextest profile: default
    Starting 12 tests across 4 binaries
        PASS [   0.004s] ( 1/12) eightball::question validate_rejects
        PASS [   0.004s] ( 2/12) eightball::question score_labels
        PASS [   0.004s] ( 3/12) eightball::config minimal_ok
        PASS [   0.004s] ( 4/12) eightball::question choice_roundtrip_keeps_order
        PASS [   0.004s] ( 5/12) eightball::config retention_default
        PASS [   0.004s] ( 6/12) eightball::config logprob_needs_model
        PASS [   0.004s] ( 7/12) eightball::config bad_spec_name
        PASS [   0.012s] ( 8/12) eightball::config unknown_key_rejected
        PASS [   0.012s] ( 9/12) eightball::question noul_without_criteria_parses
        PASS [   0.012s] (10/12) eightball::config default_must_exist
        PASS [   0.012s] (11/12) eightball::config spec_parses
        PASS [   0.012s] (12/12) eightball::config zero_concurrency
────────────
     Summary [   0.012s] 12 tests run: 12 passed, 0 skipped
```

`cargo fmt --all` passed. All eight specified config tests pass; full suite: 12 passed. Tempfile builder uses the `8ball-` prefix to respect the allowed `/tmp/8ball-*` paths. No environment variables are changed in the test process.

### Step 4: verdict math

```text
$ cargo nextest run --test verdict (red)
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
error[E0432]: unresolved imports `eightball::verdict::Bands`, `eightball::verdict::confidence_choice`, `eightball::verdict::confidence_score`, `eightball::verdict::expected_score`, `eightball::verdict::verdict`
 --> tests/verdict.rs:2:26
  |
2 | use eightball::verdict::{Bands, confidence_choice, confidence_score, expected_score, verdict};
  |                          ^^^^^  ^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^  ^^^^^^^ no `verdict` in `verdict`
  |                          |      |                  |                 |
  |                          |      |                  |                 no `expected_score` in `verdict`
  |                          |      |                  no `confidence_score` in `verdict`
  |                          |      no `confidence_choice` in `verdict`
  |                          no `Bands` in `verdict`

For more information about this error, try `rustc --explain E0432`.
error: could not compile `eightball` (test "verdict") due to 1 previous error
error: command `/home/tanguille/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo '--color=auto' test --no-run '--message-format=json-render-diagnostics' '--test=verdict'` exited with code 101
```

```text
$ cargo nextest run --test verdict (green)
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.62s
────────────
 Nextest run ID 6b5a02e1-73e9-4a72-83f1-63a55af1d3f7 with nextest profile: default
    Starting 6 tests across 1 binary
        PASS [   0.003s] (1/6) eightball::verdict score_verdict_is_mode_index
        PASS [   0.003s] (2/6) eightball::verdict choice_default_bands
        PASS [   0.003s] (3/6) eightball::verdict choice_confidence_readme
        PASS [   0.003s] (4/6) eightball::verdict noul_default_bands
        PASS [   0.003s] (5/6) eightball::verdict uniform_is_zero
        PASS [   0.003s] (6/6) eightball::verdict score_confidence_readme
────────────
     Summary [   0.004s] 6 tests run: 6 passed, 0 skipped
```

```text
$ cargo clippy --all-targets -- -D warnings
    Checking eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.49s
```

```text
$ cargo nextest run
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.45s
────────────
 Nextest run ID f2a4f103-8098-493a-804c-d28bccf63f5e with nextest profile: default
    Starting 18 tests across 5 binaries
        PASS [   0.003s] ( 1/18) eightball::verdict noul_default_bands
        PASS [   0.003s] ( 2/18) eightball::config bad_spec_name
        PASS [   0.003s] ( 3/18) eightball::question validate_rejects
        PASS [   0.003s] ( 4/18) eightball::config retention_default
        PASS [   0.003s] ( 5/18) eightball::config default_must_exist
        PASS [   0.003s] ( 6/18) eightball::verdict choice_confidence_readme
        PASS [   0.004s] ( 7/18) eightball::question score_labels
        PASS [   0.011s] ( 8/18) eightball::verdict score_verdict_is_mode_index
        PASS [   0.011s] ( 9/18) eightball::verdict uniform_is_zero
        PASS [   0.011s] (10/18) eightball::question noul_without_criteria_parses
        PASS [   0.011s] (11/18) eightball::question choice_roundtrip_keeps_order
        PASS [   0.012s] (12/18) eightball::config unknown_key_rejected
        PASS [   0.012s] (13/18) eightball::config logprob_needs_model
        PASS [   0.012s] (14/18) eightball::config zero_concurrency
        PASS [   0.012s] (15/18) eightball::config spec_parses
        PASS [   0.012s] (16/18) eightball::config minimal_ok
        PASS [   0.012s] (17/18) eightball::verdict choice_default_bands
        PASS [   0.012s] (18/18) eightball::verdict score_confidence_readme
────────────
     Summary [   0.012s] 18 tests run: 18 passed, 0 skipped
```

`cargo fmt --all` passed. Six specified verdict tests pass; full suite: 18 passed. No deviations.

### Step 5: System One backend and live tests

```text
$ cargo nextest run --test systemone (red)
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
error[E0432]: unresolved imports `eightball::backend::Endpoint`, `eightball::backend::fit`
 --> tests/systemone.rs:2:27
  |
2 | use eightball::{backend::{Endpoint, fit}, config::EndpointConfig, question::Question};
  |                           ^^^^^^^^  ^^^ no `fit` in `backend`
  |                           |
  |                           no `Endpoint` in `backend`

For more information about this error, try `rustc --explain E0432`.
error: could not compile `eightball` (test "systemone") due to 1 previous error
error: command `/home/tanguille/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo '--color=auto' test --no-run '--message-format=json-render-diagnostics' '--test=systemone'` exited with code 101
```

```text
$ cargo nextest run --test live --run-ignored only --no-capture (red)
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
error[E0432]: unresolved import `eightball::backend::Endpoint`
 --> tests/live.rs:1:17
  |
1 | use eightball::{backend::Endpoint, config::Config, question::Question, verdict::expected_score};
  |                 ^^^^^^^^^--------
  |                          |
  |                          no `Endpoint` in `backend`

For more information about this error, try `rustc --explain E0432`.
error: could not compile `eightball` (test "live") due to 1 previous error
error: command `/home/tanguille/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo '--color=auto' test --no-run '--message-format=json-render-diagnostics' '--test=live'` exited with code 101
```

```text
$ cargo nextest run --test systemone (green)
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 2.35s
────────────
 Nextest run ID 2b7b20d0-07a1-413f-b399-709e7a27d51b with nextest profile: default
    Starting 7 tests across 1 binary
        PASS [   0.004s] (1/7) eightball::systemone fit_respects_char_boundaries
        PASS [   0.016s] (2/7) eightball::systemone three_questions_one_request
        PASS [   0.017s] (3/7) eightball::systemone long_state_is_fitted
        PASS [   0.018s] (4/7) eightball::systemone http_400_is_error_with_body
        PASS [   0.019s] (5/7) eightball::systemone http_501_is_not_decision_model
        PASS [   0.032s] (6/7) eightball::systemone bearer_only_when_set
        PASS [   0.632s] (7/7) eightball::systemone concurrency_one_serializes
────────────
     Summary [   0.633s] 7 tests run: 7 passed, 0 skipped
```

```text
$ cargo clippy --all-targets -- -D warnings
    Checking eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.85s
```

```text
$ cargo nextest run
   Compiling eightball v0.1.0 (/home/tanguille/Documents/PriveProjecten/8-ball)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.90s
────────────
 Nextest run ID ffeddb3a-09d4-41a5-ad9c-39130f7969ee with nextest profile: default
    Starting 25 tests across 7 binaries (5 tests skipped)
        PASS [   0.004s] ( 1/25) eightball::question noul_without_criteria_parses
        PASS [   0.005s] ( 2/25) eightball::config logprob_needs_model
        PASS [   0.005s] ( 3/25) eightball::config zero_concurrency
        PASS [   0.005s] ( 4/25) eightball::config minimal_ok
        PASS [   0.005s] ( 5/25) eightball::config retention_default
        PASS [   0.005s] ( 6/25) eightball::config bad_spec_name
        PASS [   0.005s] ( 7/25) eightball::question choice_roundtrip_keeps_order
        PASS [   0.005s] ( 8/25) eightball::config unknown_key_rejected
        PASS [   0.005s] ( 9/25) eightball::question validate_rejects
        PASS [   0.005s] (10/25) eightball::config default_must_exist
        PASS [   0.012s] (11/25) eightball::verdict score_confidence_readme
        PASS [   0.005s] (12/25) eightball::verdict choice_default_bands
        PASS [   0.013s] (13/25) eightball::systemone fit_respects_char_boundaries
        PASS [   0.005s] (14/25) eightball::verdict choice_confidence_readme
        PASS [   0.006s] (15/25) eightball::verdict score_verdict_is_mode_index
        PASS [   0.009s] (16/25) eightball::verdict uniform_is_zero
        PASS [   0.007s] (17/25) eightball::verdict noul_default_bands
        PASS [   0.007s] (18/25) eightball::config spec_parses
        PASS [   0.017s] (19/25) eightball::question score_labels
        PASS [   0.025s] (20/25) eightball::systemone http_400_is_error_with_body
        PASS [   0.026s] (21/25) eightball::systemone http_501_is_not_decision_model
        PASS [   0.037s] (22/25) eightball::systemone bearer_only_when_set
        PASS [   0.029s] (23/25) eightball::systemone long_state_is_fitted
        PASS [   0.037s] (24/25) eightball::systemone three_questions_one_request
        PASS [   0.658s] (25/25) eightball::systemone concurrency_one_serializes
────────────
     Summary [   0.658s] 25 tests run: 25 passed, 5 skipped
```

```text
$ cargo nextest run --test live --run-ignored only --no-capture
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.09s
────────────
 Nextest run ID 2bbe9926-9713-4d2f-b263-0e9fa39f6638 with nextest profile: default
    Starting 5 tests across 1 binary
       START [         ] (1/5) eightball::live live_systemone_choice

running 1 test
"I was charged twice for my order": [[0.9949715037022028, 0.0020815854167467985, 0.002946910881050271]]
"My parcel never arrived": [[0.03348390306046388, 0.9576809061787368, 0.008835190760799286]]
"The app crashes when I log in": [[9.56862859865622e-5, 8.838856750201063e-5, 0.9998159251465114]]
test live_systemone_choice ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.34s

        PASS [   0.348s] (1/5) eightball::live live_systemone_choice
       START [         ] (2/5) eightball::live live_systemone_long_state_fits

running 1 test
long state: [[0.8418858296564597, 0.15811417034354025]]; fitted characters 2003
test live_systemone_long_state_fits ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.56s

        PASS [   0.564s] (2/5) eightball::live live_systemone_long_state_fits
       START [         ] (3/5) eightball::live live_systemone_noul_clear_cases

running 1 test
"hi there, how are you?": [[0.9977792984110628, 0.0022207015889371684]]
"Good morning everyone!": [[0.9973104968558432, 0.002689503144156835]]
"The disk on node control-2 is 98% full.": [[0.0006734414207704911, 0.9993265585792295]]
"Invoice #4411 is attached.": [[0.0005724723698325666, 0.9994275276301674]]
test live_systemone_noul_clear_cases ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.40s

        PASS [   0.407s] (3/5) eightball::live live_systemone_noul_clear_cases
       START [         ] (4/5) eightball::live live_systemone_score

running 1 test
urgent: [[0.0035533644092976225, 0.0024632714944271077, 0.09199483747399864, 0.9019885266222766]], score 2.892418526309254; typo: [[0.8658067956293518, 0.016835738079480713, 0.0534055786459527, 0.06395188764521488]], score 0.3155025583070308
test live_systemone_score ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.22s

        PASS [   0.228s] (4/5) eightball::live live_systemone_score
       START [         ] (5/5) eightball::live live_systemone_three_questions_one_call

running 1 test
multi: [[0.001626082216226916, 0.998373917783773], [0.9930987578168688, 0.00038615523376224806, 0.006515086949368796], [0.05358819608051216, 0.013706931575184388, 0.47920685379576855, 0.4534980185485349]]
test live_systemone_three_questions_one_call ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.31s

        PASS [   0.315s] (5/5) eightball::live live_systemone_three_questions_one_call
────────────
     Summary [   1.863s] 5 tests run: 5 passed, 0 skipped
```

`cargo fmt --all` passed. Mock suite: 25 passed, 5 ignored live tests. Explicit live run: 5 passed. The fitted state contains 2000 input characters plus the three-character delimiter (2003 total), matching the specified fit rule. Deviation: the unused private `extra_body` endpoint field is deferred to Chunk 2, where the logprob backend consumes it, to keep the required clippy check clean.
