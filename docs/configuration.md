# Configuration

The default path is `$XDG_CONFIG_HOME/8ball/config.toml`, otherwise `$HOME/.config/8ball/config.toml`. 8-ball rejects unknown fields at the top level, in `[endpoints.*]`, `[embedder]`, and `[learning]`. It ignores unknown fields in `[specs.*]`, but rejects invalid questions.

```toml
default = "d3"            # required: name of the endpoint used when none is given
retention_days = 90       # optional (default 90): unlabelled decisions older than this are deleted at startup

[endpoints.d3]
url = "http://127.0.0.1:9931"   # required: server root. A trailing "/v1" or "/" is removed.
kind = "systemone"              # optional: "systemone" | "logprob" | "rerank". Omitted = auto-detect.
concurrency = 1                 # optional: max parallel requests. Omitted = /props total_slots, else 2.
model = "qwen-3.8-fast"         # optional: sent as "model". Required for kind logprob.
api_key_env = "TYPESAFE_KEY"    # optional: NAME of an env var whose value is sent as "Authorization: Bearer <value>"
max_state_chars = 2000          # optional: default 2000 for systemone and rerank, no limit for logprob
extra_body = { reasoning_effort = "none" }   # optional: merged into logprob request JSON

[embedder]                      # optional table
url = "http://127.0.0.1:9932"
model = "embeddinggemma-2"

[specs.greeting]                # optional, any number of specs; the name must match ^[a-z0-9][a-z0-9-]{0,63}$
type = "noul"
instructions = "Is this message a greeting?"
# criteria = { true = "it greets", false = "it does not" }    # noul: optional
# criteria = { billing = "", shipping = "Parcels" }          # choice: required, ≥2 options, order kept, "" = no description
# criteria = ["can wait", "today", "now"]                    # score: required, 2..=10 levels, lowest first
# endpoint = "d3"                                            # optional, else `default`

[learning]                      # optional table; each part can be turned off
report = true                   # mark decisions whose answer the agent should report with `label`
remember = true                 # answer with a reported label when the same question and state come back
explore = 0.05                  # share of skipped decisions returned as `unsure`, so the agent checks them; 0 to 1
recalibrate_after = 20          # refit a question's bands at every multiple of this many labels; 0 turns it off
ad_hoc = true                   # also learn ad-hoc questions, by their exact text
```

`[embedder]` also takes an optional `api_key_env`. `concurrency` must be positive. `rerank` supports noul only. A reranker scores how well the state matches the topic of the question, not if the answer is yes. So its answers stay `unsure` until you [calibrate](calibration.md) the question. Auto-detection reads `/props` and `/v1/models`. If a state exceeds `max_state_chars`, 8-ball removes its middle. The order of choice options and score levels matters.

## Environment variables

A flag overrides its variable.

| Variable | Flag | Purpose |
| --- | --- | --- |
| `EIGHTBALL_CONFIG` | `--config` | Configuration path for `serve` and `mcp` |
| `EIGHTBALL_CONFIG_TOML` | `--config-toml` | Configuration TOML text for `serve` and `mcp`, instead of a file. You cannot also set `EIGHTBALL_CONFIG`. |
| `EIGHTBALL_LISTEN` | `--listen` | `serve` listen address (default `127.0.0.1:8080`, image default `0.0.0.0:8080`) |
| `EIGHTBALL_DB` | `--db` | SQLite path for `serve` and `mcp` (default `$XDG_STATE_HOME/8ball/8ball.db`, otherwise `$HOME/.local/state/8ball/8ball.db`) |
| `EIGHTBALL_API_KEY` | `--api-key` | Bearer key for the server and the client |
| `EIGHTBALL_URL` | `--url` | CLI server root (default `http://127.0.0.1:8080`) |
| `RUST_LOG` | None | Filter for the logs on stderr (default `info,rmcp=warn`) |

## Thinking chat models

The logprob backend needs a one-token answer, so you must disable reasoning. Set `extra_body = { reasoning_effort = "none" }`. For Qwen on vLLM, set `extra_body = { chat_template_kwargs = { enable_thinking = false } }`. The pinned local Qwen server also starts with `--reasoning-budget 0`.
