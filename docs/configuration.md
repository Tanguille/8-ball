# Configuration

8-ball looks for `$XDG_CONFIG_HOME/8ball/config.toml` by default. If `XDG_CONFIG_HOME` is unset, it looks for `$HOME/.config/8ball/config.toml`.

An endpoint is a model server. A spec is a saved question.

8-ball rejects unknown top-level fields and unknown fields in `[endpoints.*]`, `[embedder]`, and `[learning]`. In `[specs.*]`, it ignores unknown fields and still checks that each question is valid.

```toml
default = "d3"            # required: name of the endpoint used when none is given
retention_days = 90       # optional (default 90): unlabelled decisions older than this are deleted at startup, unless the same question and state has a label

[endpoints.d3]
url = "http://127.0.0.1:9931"   # required: server root. A trailing "/v1" or "/" is removed.
kind = "systemone"              # optional: "systemone" | "decisions" | "logprob" | "rerank". Omitted = auto-detect.
concurrency = 1                 # optional: max parallel requests. Omitted = /props total_slots if positive, else 2.
model = "qwen-3.8-fast"         # optional: sent as "model". Required for kind logprob.
api_key_env = "TYPESAFE_KEY"    # optional: NAME of an env var whose value is sent as "Authorization: Bearer <value>"
max_state_chars = 2000          # optional: default 2000 for systemone, decisions and rerank, no limit for logprob
timeout_secs = 20               # optional: seconds per HTTP attempt once it has a slot (a retry gets its own); waiting for a slot is not counted
extra_body = { reasoning_effort = "none" }   # optional: merged into logprob request JSON
requests_per_minute = 30        # optional: space requests evenly, for a hosted provider's rate limit; must be positive

[embedder]                      # optional table
url = "http://127.0.0.1:9932"
model = "embeddinggemma-2"

[specs.greeting]                # optional, any number of specs; the name must match ^[a-z0-9][a-z0-9-]{0,63}$
type = "noul"
instructions = "Is this message a greeting?"
# criteria = { true = "it greets", false = "it does not" }    # noul: optional
# criteria = { billing = "", shipping = "Parcels" }          # choice: required, ≥2 options, order kept, "" = no description, "unsure" is reserved
# criteria = ["can wait", "today", "now"]                    # score: required, 2..=10 levels, lowest first
# endpoint = "d3"                                            # optional, else `default`

[learning]                      # optional table; each part can be turned off
report = true                   # mark decisions whose answer the agent should report with `label`
remember = true                 # answer with a reported label when the same question and state come back
explore = 0.05                  # share of skipped decisions returned as `unsure`, so the agent checks them; 0 to 1
recalibrate_after = 20          # refit an endpoint's bands each time its labelled states pass a multiple of this; 0 turns it off
ad_hoc = true                   # also learn ad-hoc questions, by their exact text
```

`[embedder]` also accepts `api_key_env`. Set `concurrency` to a positive number. If you omit `kind`, 8-ball reads `/props` and `/v1/models` to detect the server type.

The `decisions` backend sends System One questions and `model` to a hosted `/v1/decisions` API. If the hosted catalog lists each model's `endpoints`, 8-ball picks `decisions` or `logprob` for the configured `model`.

If the first detection fails to connect, times out, or gets 502 or 503 from `/props`, the request fails. The next request tries again. If a later check fails, 8-ball keeps the known model.

8-ball retries a request once after a 502 or 503 response. It also retries a 429 response if `Retry-After` is at most 10 seconds. It waits at least 1 second before the 429 retry.

A reranker scores how well input text matches the topic of a question. The `rerank` backend only supports yes-or-no questions (`noul`). It returns `unsure` until you [calibrate](calibration.md) the question.

If the input text is longer than `max_state_chars`, 8-ball removes text from the middle. For `choice` and `score`, the order of options and levels matters.

## Escalation

```toml
[escalation]                    # optional table
endpoint = "qwen"               # required: a configured endpoint other than `default`
per_minute = 30                 # optional (default 30): model re-asks per minute, across all requests; cached ones are free

[specs.gitops-investigate]
escalate = false                # optional: keep this spec's `unsure` answers
```

When `[escalation]` exists, 8-ball re-asks every `unsure` answer on the escalation endpoint before it returns. A request's `escalate` field wins, then the spec's `escalate`, else escalation is on. Without `[escalation]`, it is always off. A remembered answer does not escalate. The escalation endpoint judges the same trimmed state that the default endpoint saw. Set its `max_state_chars` to at least the default endpoint's limit. A smaller limit trims the state again, so one label can no longer serve both endpoints.

An escalation that fails, times out, or goes over `per_minute` keeps the `unsure` answer and logs a warning. The request deadlines also bound it; see [Decisions](http-api.md#decisions).

The escalation endpoint is usually a chat model with `kind = "logprob"` and thinking off, for example the 27B Qwen with `extra_body = { chat_template_kwargs = { enable_thinking = false } }`. Do not use a router that can turn thinking on: the first token is then `<think>`, and the logprob answer breaks.

## Environment variables

A flag takes priority over its environment variable.

| Variable | Flag | Purpose |
| --- | --- | --- |
| `EIGHTBALL_CONFIG` | `--config` | Configuration path for `serve` and `mcp` |
| `EIGHTBALL_CONFIG_TOML` | `--config-toml` | TOML text for `serve` and `mcp`. Leave `EIGHTBALL_CONFIG` unset when you use this variable. |
| `EIGHTBALL_LISTEN` | `--listen` | `serve` listen address (default `127.0.0.1:8080`, image default `0.0.0.0:8080`) |
| `EIGHTBALL_DB` | `--db` | SQLite path for `serve` and `mcp` (default `$XDG_STATE_HOME/8ball/8ball.db`, otherwise `$HOME/.local/state/8ball/8ball.db`) |
| `EIGHTBALL_API_KEY` | `--api-key` | Bearer key for the server and the client. `serve` rejects an empty key. |
| `EIGHTBALL_URL` | `--url` | CLI server root (default `http://127.0.0.1:8080`) |
| `RUST_LOG` | None | Filter for the logs on stderr (default `info,rmcp=warn`) |

## Thinking chat models

The `logprob` backend needs a one-token answer. Turn off reasoning with `extra_body = { reasoning_effort = "none" }`.

For Qwen on vLLM, set `extra_body = { chat_template_kwargs = { enable_thinking = false } }`. The local Qwen server in the development setup also starts with `--reasoning-budget 0`.
