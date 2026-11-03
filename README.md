# 8-ball

8-ball makes cheap typed judgments through small decision models: yes/no, pick one, or a level. It exposes REST, HTTP MCP, stdio MCP and a CLI, with optional labelled calibration and a SQLite answer cache. Use clear verdicts to gate or triage work; on `unsure` or an error, do the normal work.

## Quickstart: local

```sh
cargo install --path .
mkdir -p ~/.config/8ball
```

Write `~/.config/8ball/config.toml`:

```toml
default = "d3"

[endpoints.d3]
url = "http://127.0.0.1:9931"

[specs.greeting]
type = "noul"
instructions = "Is this message a greeting?"
```

Start a compatible model server first. This repository includes four pinned local development models:

```sh
dev/models.sh up
dev/models.sh status
8ball serve
```

In another terminal:

```sh
8ball ask noul "Is this a greeting?" --state hi
8ball ask --spec greeting --state "hey there"
```

The dev stack downloads about 4.5 GB to `~/.cache/8ball-models`. Use `EIGHTBALL_DEV_NGL=0 dev/models.sh up` for CPU inference. Stop it with `dev/models.sh down`.

## Quickstart: shared server

```sh
export EIGHTBALL_URL=https://your-8ball-server.example
export EIGHTBALL_API_KEY=your-server-token
8ball doctor
8ball ask --spec greeting --state hi
```

Use the server root without a trailing slash. On the server, `EIGHTBALL_API_KEY` enables bearer authentication for REST and MCP; health and metrics remain public. The client uses the same variable for its header.

## Commands

- `serve`: run REST and HTTP MCP on `127.0.0.1:8080`; use `--config`, `--db` or `--listen` to override paths/address.
- `mcp`: serve MCP over stdin/stdout; logs go to stderr.
- `ask`: ask an ad-hoc noul/choice/score question or a named `--spec`; read state from `--state` or stdin.
- `triage`: judge a JSON array from stdin, optionally using `--top-k` embedding prefiltering.
- `label <id> <value>`: label a saved decision belonging to a current named spec.
- `calibrate <spec>`: fit bands from labelled states; `--dry-run` reports without saving.
- `doctor`: inspect endpoint detection, health, model fingerprints and spec calibration.
- `config check <path>`: validate TOML without starting the service.

`results[].id` is a saved decision row id for labelling; `results[].question` is the caller's question id. Repeated identical requests within the same spec reuse a row; ad-hoc and different named specs have separate rows. Invalid backend probabilities return a backend error.

## Config reference

Default: `$XDG_CONFIG_HOME/8ball/config.toml`, otherwise `$HOME/.config/8ball/config.toml`. Flags override environment defaults. Unknown fields and invalid questions are rejected.

```toml
default = "d3"            # required: name of the endpoint used when none is given
retention_days = 90       # optional (default 90): unlabelled decisions older than this are deleted at startup

[endpoints.d3]
url = "http://127.0.0.1:9931"   # required: server root. A trailing "/v1" or "/" is removed.
kind = "systemone"              # optional: "systemone" | "logprob" | "rerank". Omitted = auto-detect (chunk 2).
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
```

Choice option order and score level order matter. Rerank supports noul only. Auto-detection reads `/props` and `/v1/models`; explicit logprob endpoints require `model`. The default database path is `$XDG_STATE_HOME/8ball/8ball.db`, otherwise `$HOME/.local/state/8ball/8ball.db`.

## Environment variables

| Variable | Purpose |
| --- | --- |
| `EIGHTBALL_CONFIG` | Server/stdio config path |
| `EIGHTBALL_LISTEN` | Server listen address (default `127.0.0.1:8080`) |
| `EIGHTBALL_DB` | SQLite database path |
| `EIGHTBALL_API_KEY` | Server bearer key and client bearer credential |
| `EIGHTBALL_URL` | CLI server root (default `http://127.0.0.1:8080`) |

Endpoint `api_key_env` names a separate variable for authenticating to that model backend.

## Calibration

Save a question as a reviewed config spec, then label real decisions using their numeric row ids.
Aim for at least 20 examples per side before running `8ball calibrate <spec>`.
Bands select which answers are decisive at the requested precision; the rest are `unsure`.
A dry run leaves stored bands untouched; reports are in-sample, so evaluate a separate holdout.
Changing the question or model invalidates its calibration identity; ad-hoc questions always use defaults.

## Thinking chat models

The logprob backend needs reasoning disabled for a one-token answer. Set `extra_body = { reasoning_effort = "none" }`, or for Qwen on vLLM use `extra_body = { chat_template_kwargs = { enable_thinking = false } }`. The pinned local Qwen server also starts with `--reasoning-budget 0`.

## Privacy

The database stores the raw text of every successful decision and is created with mode 0600. Unlabelled rows older than `retention_days` are deleted at startup; labelled rows remain. The intended cluster deployment uses kopiur to copy the PVC to R2; cluster deployment and backups are outside this local build. Model requests go to your configured endpoints, and harness prompts also go to the harness's configured model provider.

[Privacy Policy](README.md#privacy) describes this local plugin's data handling. Review a shared server operator's policy before sending private data.

## Claude Code

**Install**

```sh
claude plugin marketplace add /home/tanguille/Documents/PriveProjecten/8-ball
claude plugin install 8-ball@8-ball
```

The plugin uses `EIGHTBALL_URL` and `EIGHTBALL_API_KEY`. With a local keyless server, the empty bearer header is accepted.

**Verify**

```sh
claude -p --plugin-dir ./plugin \
  --allowedTools 'mcp__plugin_8-ball_8ball__decision_ask,mcp__plugin_8-ball_8ball__decision_triage' \
  "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
curl -s http://127.0.0.1:8080/metrics
```

Tested on 2026-11-03 with Claude Code 2.1.317: actual plugin tool call, reported verdict and increased decision counter. Strict standalone `--mcp-config plugin/.mcp.json` uses the `mcp__8ball__` prefix instead.

**Uninstall**

```sh
claude plugin uninstall 8-ball@8-ball
```

## Codex

**Install**

```sh
codex plugin marketplace add /home/tanguille/Documents/PriveProjecten/8-ball
codex plugin add 8-ball@8-ball
```

For a shared server, edit `plugin/.mcp.codex.json`'s URL before installation: Codex does not expand environment variables in URLs. The key is read from `EIGHTBALL_API_KEY`.

Fallback:

```sh
codex mcp add 8ball --url http://127.0.0.1:8080/mcp --bearer-token-env-var EIGHTBALL_API_KEY
ln -s /home/tanguille/Documents/PriveProjecten/8-ball/plugin/skills/8-ball ~/.agents/skills/8-ball
```

**Verify**

```sh
codex exec --ephemeral \
  -c 'mcp_servers.8ball.url="http://127.0.0.1:8080/mcp"' \
  -c 'mcp_servers.8ball.tools.decision_ask.approval_mode="approve"' \
  'Call decision_ask with {"state":"disk 98% full on /var","questions":[{"id":"urgent","type":"noul","instructions":"Is this alert urgent?"}]}, then report the verdict.'
```

The one-tool approval override applies only to this synthetic local test; normal installed sessions can approve calls interactively. [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) documents per-tool approval settings.

Tested on 2026-11-03 with Codex CLI 0.166.0: HTTP MCP tool call completed, reported `unsure`, and increased the decision counter. This tested the per-run MCP fallback; persistent marketplace installation was not performed.

**Uninstall**

```sh
codex plugin remove 8-ball@8-ball
```

For the fallback, run `codex mcp remove 8ball` and remove only the skill symlink you created.

## Cursor

**Install**

Use this repository's `.cursor-plugin/marketplace.json` in Cursor's plugin marketplace, or copy the `mcpServers.8ball` entry from `plugin/.mcp.cursor.json` into `~/.cursor/mcp.json`. Adjust its URL for a shared server; the key uses `${env:EIGHTBALL_API_KEY}`.

**Verify**

```sh
cursor-agent --print --plugin-dir ./plugin --approve-mcps \
  "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

Untested: the installed Cursor Agent 2026.07.23-e383d2b accepts a per-run plugin, but the isolated test exited with authentication required before connecting. Pending user: `cursor-agent login`, then the command above. Check that a tool call occurs and the server's decision counter increases.

**Uninstall**

Remove the 8-ball plugin in Cursor's marketplace, or remove only the copied `mcpServers.8ball` entry and any skill link you added.

## qwen-code

**Install**

Untested: qwen-code is not installed; installing tools is outside this local run. Pending user:

```sh
npm install -g @qwen-code/qwen-code@latest
qwen extensions install /home/tanguille/Documents/PriveProjecten/8-ball/plugin
```

**Verify**

```sh
qwen -p "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

Adjust `plugin/qwen-extension.json`'s `httpUrl` for a shared server. Its header reads `$EIGHTBALL_API_KEY`. The [extension format](https://qwenlm.github.io/qwen-code-docs/en/users/extension/introduction/) and [HTTP transport fields](https://qwenlm.github.io/qwen-code-docs/en/users/features/mcp/) were checked; runtime behavior remains untested.

**Uninstall**

```sh
qwen extensions uninstall 8-ball
```

## OpenCode v2

**Install**

Untested: this machine has v1; replacing it is outside this local run. Pending user: run `npm install -g @opencode/cli@latest` ([official v2 instructions](https://opencode.ai/v2/docs/)), then add the following to `~/.config/opencode/opencode.json`:

```json
{"mcp":{"servers":{"8ball":{"type":"remote","url":"http://127.0.0.1:8080/mcp","oauth":false,"headers":{"Authorization":"Bearer {env:EIGHTBALL_API_KEY}"}}}}}
```

```sh
ln -s /home/tanguille/Documents/PriveProjecten/8-ball/plugin/skills/8-ball ~/.agents/skills/8-ball
```

[Official MCP docs](https://opencode.ai/v2/docs/mcp-servers) confirm `mcp.servers` and `oauth: false` for bearer-only authentication. Keep the default Code Mode initially.

**Verify**

```sh
opencode mcp list
opencode run "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

Confirm a decision tool call and metric increase. If Code Mode cannot call it, try `"codemode": false` in that server entry and record the failure first.

**Uninstall**

Remove only `mcp.servers.8ball` from the config and the skill symlink you created.

## Hermes

Untested and deferred: deployment, shared MCP registration and Hermes gates belong to Chunks 5 and 7, outside this local run. No Hermes configuration was changed.

## Development verification

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo nextest run
dev/models.sh status
cargo nextest run --test live --run-ignored only
docker build -t 8ball:dev .
docker run --rm 8ball:dev --version
claude plugin validate plugin --strict
```

Build with current stable Rust; `Cargo.lock` records tested dependency versions. Latest versions reduce upgrade backlog but do not guarantee freedom from breaking changes. Real-model probabilities, holdout results, deviations and local-only deferrals are recorded in [the execution report](docs/plans/8-ball-v1-report.md).
