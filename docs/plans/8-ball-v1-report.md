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
