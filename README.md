# 8-ball

> [!WARNING]
> 8-ball is in early development. Commands, APIs, configuration, and the database format can change without notice. Use it at your own risk.

8-ball gets cheap typed judgments from small decision models. A question is `noul` (yes or no), `choice` (pick one option), or `score` (pick a level). The verdict is the answer to a question. 8-ball offers REST, HTTP MCP, stdio MCP, and a CLI, with a SQLite answer cache and optional labeled calibration. Use a clear verdict to gate or triage work. If the verdict is `unsure` or an error occurs, do the normal work.

8-ball is a Swiss Army knife that lets agents get the most out of decision models in a simple way. It is a Rust server and CLI. It uses System One decision models and rerankers on llama.cpp servers, and chat models through logprobs on any OpenAI-compatible server that returns them, such as vLLM. AI agents in Claude Code, Codex, Cursor, qwen-code, and OpenCode can connect to it as an MCP server. A decision model then answers simple classification questions before the agent does the expensive work.

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

The dev stack downloads about 4.5 GB to `~/.cache/8ball-models`. For CPU inference, use `EIGHTBALL_DEV_NGL=0 dev/models.sh up`. To stop the stack, use `dev/models.sh down`. The stack works with Docker and with rootless Podman, also with SELinux enforcing.

## Quickstart: shared server

```sh
export EIGHTBALL_URL=https://your-8ball-server.example
export EIGHTBALL_API_KEY=your-server-token
8ball doctor
8ball ask --spec greeting --state hi
```

Use the server root without a trailing slash.

## Examples

These results come from the pinned development models: `dev/models.sh up`, then `8ball serve --config dev/config.toml`, on the first run with an empty database. The bands are the defaults, without calibration. Other models, hardware, and versions give other numbers. Numbers can also change a little between runs.

Ask a spec. Each command prints one JSON line, and `jq` formats it:

```sh
8ball ask --spec greeting --state "hey there, how are you?" | jq
```

```json
{
  "results": [
    {
      "id": 1,
      "question": "greeting",
      "type": "noul",
      "verdict": "yes",
      "confidence": 0.994583630474132,
      "probabilities": {
        "yes": 0.997291815237066,
        "no": 0.002708184762933996
      },
      "calibrated": false,
      "cached": false
    }
  ]
}
```

`confidence` shows how sure the model is. The bands turn it into a verdict. If the confidence is too low, the verdict is `unsure`, and you do the normal work.

Ask a choice or a score question without a spec:

```sh
8ball ask choice "Which team should handle this ticket?" \
  --option billing="Invoices, payments and refunds" \
  --option shipping="Parcels and delivery" \
  --option security="Accounts and access" \
  --state "I was charged twice for my subscription this month."

8ball ask score "How urgent is this alert?" \
  --level "can wait" --level today --level now \
  --state "Primary database is down, all writes fail"
```

A score verdict is the index of the level, lowest first. More results, with the confidence rounded:

| Question | State | Verdict | Confidence |
| --- | --- | --- | --- |
| `--spec storage-risk` | Backups have failed for the last 4 nights | `yes` | 0.98 |
| `--spec storage-risk` | The weekly report is ready to download | `no` | 0.91 |
| `noul "Does this alert need a human now?"` | disk 98% full on /var | `unsure` | 0.63 |
| The choice above | I was charged twice for my subscription this month. | `billing` | 0.99 |
| The choice above | My parcel arrived damaged and I want a replacement. | `shipping` | 0.99 |
| The choice above | Someone logged into my account from another country. | `security` | 1.00 |
| The score above | Primary database is down, all writes fail | `2` (now) | 0.93 |
| The score above | Certificate expires in 20 days | `unsure` | 0.34 |
| The score above | Weekly report is ready | `0` (can wait) | 0.61 |

Triage a list with one spec. The output keeps the original indices:

```sh
echo '["Primary database is down, all writes fail", "Weekly report is ready", "Backups have failed for the last 4 nights", "New comment on your blog post"]' \
  | 8ball triage --spec storage-risk \
  | jq -c '{kept: [.kept[].index], rejected: [.rejected[].index], unsure: [.unsure[].index]}'
```

```json
{"kept":[0,2],"rejected":[1,3],"unsure":[]}
```

Item 2 came from the answer cache, because the table above asked the same spec and state.

Gate a shell script with `--exit-code`:

```sh
8ball ask --spec greeting --state "hey there, how are you?" --exit-code > /dev/null; echo $?                    # 0: yes
8ball ask --spec greeting --state "Please send me the invoice for October." --exit-code > /dev/null; echo $?    # 1: no
8ball ask noul "Does this alert need a human now?" --state "Weekly report is ready" --exit-code > /dev/null; echo $?   # 3: unsure
```

Agents get the same verdicts through the MCP tools `decision_ask` and `decision_triage`.

## Commands

A spec is a named question in the configuration. The state is the text to judge. An endpoint is a configured model server. Its `kind` is the backend, which sets how 8-ball talks to that server. All commands except `serve`, `mcp`, and `config check` call a running server and refuse HTTP redirects.

| Command | Purpose |
| --- | --- |
| `serve` | Run REST and HTTP MCP. |
| `mcp` | Serve MCP over stdin and stdout. |
| `ask [<kind> <instructions>]` | Ask one ad-hoc question or a `--spec`. `--state` gives the state. The default is stdin. `--exit-code` enables exit codes 1 and 3. |
| `triage [<instructions>]` | Judge a JSON array from stdin with one question or a `--spec`, not both. `--type` sets the question type (default `noul`). `--context-file` names a UTF-8 file with shared evidence for every item. `--top-k` judges only the items most similar to the question and needs `[embedder]`. |
| `observe` | Record the triage decisions of a snapshot in a private file. Print JSON with `wakeAgent: true` and every candidate. It requires `--spec`, `--input` (JSON with `context` text and at most 64 `items`), and `--observations` (a new JSONL file, never overwritten). |
| `evaluate` | Replay labeled JSONL cases from stdin against a required `--spec`. Report misses, `unsure` verdicts, errors, and latency. Calibration does not change. |
| `label <id> <value>` | Label a saved decision of a current spec. The value is `yes`, `no`, an option name, or a level index. |
| `calibrate <spec>` | Fit bands (decision thresholds) from labeled states. `--precision` takes 0.5 to 0.99 and defaults to 0.95. `--dry-run` reports without a save. |
| `doctor` | Report endpoint detection, health, model fingerprints, and spec calibration. |
| `config check [<path>]` | Make sure that a TOML file is valid, without a server. `<path>` defaults to the [configuration path](#configuration-reference), not `EIGHTBALL_CONFIG`. |

| Shared flag | Commands | Purpose |
| --- | --- | --- |
| `--endpoint <name>` | `ask`, `triage`, `evaluate`, `observe` | Override the endpoint. |
| `--option <name[=description]>`, `--level <text>` | `ask`, `triage` | Add a choice option or a score level, lowest first. Both repeat. |
| `--keep <value>` | `triage`, `evaluate` | Keep `yes` (default), `no`, an option name, or a minimum level index. The default works for noul only, so choice and score questions need `--keep`. |
| `--timeout-ms <ms>` | All that call a server | Total request deadline, at least 1. The default is 25,000 for `observe` and `evaluate`, otherwise none. |

[Environment variables](#environment-variables) lists the other flags. `8ball --version` prints the version.

| Exit code | Meaning |
| --- | --- |
| 0 | Success. With `--exit-code`, the verdict is `yes`, an option, or a level. |
| 1 | With `--exit-code`, the noul verdict is `no`. |
| 2 | The argument parser rejected the arguments. |
| 3 | With `--exit-code`, the verdict is `unsure`. |
| 4 | Any other error, for example `ask` with neither `--spec` nor `<kind> <instructions>`. For `doctor`, the default endpoint failed. For `evaluate`, a case failed. |

`observe` exits 0 after input, model, and write errors. Its JSON reports them.

`results[].id` is the saved row id for `label`. `results[].question` is the question id of the caller. `results[].type` is the question type. An identical request within a spec reuses a row. Ad-hoc questions and different specs have separate rows. Invalid backend probabilities return a backend error.

## HTTP routes

If `EIGHTBALL_API_KEY` is set on the server, the protected routes need `Authorization: Bearer <key>`. A missing or wrong key returns 401. Without a key, `/mcp` accepts only the hosts `localhost`, `127.0.0.1`, and `::1`.

| Route | Protected | Purpose |
| --- | --- | --- |
| `POST /v1/ask`, `POST /v1/triage` | Yes | Take the input of `decision_ask` and `decision_triage`. |
| `POST /v1/label`, `POST /v1/calibrate` | Yes | Take the CLI arguments as JSON: `id`, `value`, `spec`, `precision`, `dry_run`. |
| `GET /v1/doctor` | Yes | Return the `doctor` report. |
| `/mcp` | Yes | Serve HTTP MCP. |
| `GET /healthz` | No | Return `ok`. |
| `GET /metrics` | No | Return the OpenMetrics counters `eightball_decisions_total` and `eightball_backend_errors_total`. |
| `POST /register`, `GET /.well-known/{*rest}` | No | Return 404, because 8-ball does not use OAuth. |

Service and authentication errors are JSON with an `error` field. Rejected request bodies and rejected `/mcp` hosts return plain text.

## MCP tools

| Tool | Purpose |
| --- | --- |
| `decision_ask` | Ask typed questions about `state`. It takes `spec` or `questions`, and an optional `endpoint`. |
| `decision_triage` | Judge at most 64 `items` with one `spec` or `question`. It also takes optional `context`, `keep`, `top_k`, and `endpoint`. |

## Configuration reference

The default path is `$XDG_CONFIG_HOME/8ball/config.toml`, otherwise `$HOME/.config/8ball/config.toml`. 8-ball rejects unknown fields at the top level, in `[endpoints.*]`, and in `[embedder]`. It ignores unknown fields in `[specs.*]`, but rejects invalid questions.

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
```

`[embedder]` also takes an optional `api_key_env`. `concurrency` must be positive. `rerank` supports noul only. Auto-detection reads `/props` and `/v1/models`. If a state exceeds `max_state_chars`, 8-ball removes its middle. The order of choice options and score levels matters.

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
| `RUST_LOG` | None | Filter for the logs on stderr (default `info`) |

## Calibration

Bands select the decisive verdicts at the requested precision. The other verdicts are `unsure`.

To calibrate a spec:

1. Save the question as a reviewed spec in the configuration.
2. Label at least 20 real decisions per side with their numeric row ids.
3. Run `8ball calibrate <spec>`.

Reports are in-sample, so evaluate a separate holdout (labeled cases that calibration did not use). A change to the question or the model invalidates its calibration identity. Ad-hoc questions always use the default bands.

## Hermes observation pilot

[Contextual triage and evaluation](docs/hermes-triage.md) has a GitOps digest pre-check example and a holdout replay workflow. The example records decisions and passes every candidate to Hermes. It does not modify or deploy Hermes.

## Thinking chat models

The logprob backend needs a one-token answer, so you must disable reasoning. Set `extra_body = { reasoning_effort = "none" }`. For Qwen on vLLM, set `extra_body = { chat_template_kwargs = { enable_thinking = false } }`. The pinned local Qwen server also starts with `--reasoning-budget 0`.

## Privacy

Storage uses SQLx with bundled SQLite, so you need no database server or SQLx CLI. Startup applies embedded migrations and keeps decisions, labels, calibrations, and id counters, including from the original v1 database. Before an upgrade, back up an existing database with a SQLite-aware tool. This backup includes committed data in the WAL (the SQLite write-ahead log). SQLite is the only supported database. PostgreSQL and persistent vector search are future work.

The database stores the raw text of every successful decision, with file mode 0600. `retention_days` never deletes labeled rows. The [Helm chart](#helm-chart) is in this repository. Backups are outside it. Model requests go to your configured endpoints. Prompts in an MCP client also go to the model provider of that client.

Before you send private data to a shared server, read the policy of its operator.

## MCP clients

Run the commands in this section from the repository root. `EIGHTBALL_URL` configures the CLI and the Claude plugin. The other clients need the URL edits in their sections.

### Claude Code

To install:

```sh
claude plugin marketplace add .
claude plugin install 8-ball@8-ball
```

The plugin also uses `EIGHTBALL_API_KEY`. A local server with no key accepts the empty bearer header.

To test:

```sh
claude -p --plugin-dir ./plugin \
  --allowedTools 'mcp__plugin_8-ball_8ball__decision_ask,mcp__plugin_8-ball_8ball__decision_triage' \
  "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
curl -s http://127.0.0.1:8080/metrics
```

We tested this on 2026-11-03 with Claude Code 2.1.317. The plugin tool call reported a verdict and increased the decision counter. Strict standalone `--mcp-config plugin/.mcp.json` uses the `mcp__8ball__` prefix instead.

To uninstall:

```sh
claude plugin uninstall 8-ball@8-ball
```

### Codex

To install:

```sh
codex plugin marketplace add .
codex plugin add 8-ball@8-ball
```

For a shared server, edit the URL in `plugin/.mcp.codex.json` before installation, because Codex does not expand environment variables in URLs. The key comes from `EIGHTBALL_API_KEY`.

Fallback:

```sh
codex mcp add 8ball --url http://127.0.0.1:8080/mcp --bearer-token-env-var EIGHTBALL_API_KEY
ln -s "$PWD/plugin/skills/8-ball" ~/.agents/skills/8-ball
```

To test:

```sh
codex exec --ephemeral \
  -c 'mcp_servers.8ball.url="http://127.0.0.1:8080/mcp"' \
  -c 'mcp_servers.8ball.tools.decision_ask.approval_mode="approve"' \
  'Call decision_ask with {"state":"disk 98% full on /var","questions":[{"id":"urgent","type":"noul","instructions":"Is this alert urgent?"}]}, then report the verdict.'
```

The one-tool approval override applies only to this synthetic local test. Normal installed sessions can approve calls interactively. [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) documents approval for each tool.

We tested this on 2026-11-03 with Codex CLI 0.166.0 and the per-run MCP fallback, not a persistent marketplace installation. The HTTP MCP tool call reported `unsure` and increased the decision counter.

To uninstall:

```sh
codex plugin remove 8-ball@8-ball
```

For the fallback, run `codex mcp remove 8ball`. Then remove only the skill symlink that you created.

### Cursor

To install, use `.cursor-plugin/marketplace.json` in the Cursor plugin marketplace. Or copy the `mcpServers.8ball` entry from `plugin/.mcp.cursor.json` into `~/.cursor/mcp.json`. For a shared server, adjust its URL. The key uses `${env:EIGHTBALL_API_KEY}`.

To test:

```sh
cursor-agent --print --plugin-dir ./plugin --approve-mcps \
  "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

This is untested. Cursor Agent 2026.07.23-e383d2b accepts a per-run plugin, but the isolated test exited with authentication required before it connected. To try it, run `cursor-agent login`, then the command above. Make sure that a tool call occurs and that the decision counter of the server increases.

To uninstall, remove the 8-ball plugin in the Cursor marketplace. Or remove only the copied `mcpServers.8ball` entry and any skill link that you added.

### qwen-code

This is untested. To try it:

```sh
npm install -g @qwen-code/qwen-code@latest
qwen extensions install ./plugin
```

To test:

```sh
qwen -p "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

For a shared server, adjust `httpUrl` in `plugin/qwen-extension.json`. Its header reads `$EIGHTBALL_API_KEY`. A review compared the plugin with the [extension format](https://qwenlm.github.io/qwen-code-docs/en/users/extension/introduction/) and [HTTP transport fields](https://qwenlm.github.io/qwen-code-docs/en/users/features/mcp/). Runtime behavior is untested.

To uninstall:

```sh
qwen extensions uninstall 8-ball
```

### OpenCode v2

This is untested on v2. To try it, run `npm install -g @opencode/cli@latest` ([official v2 instructions](https://opencode.ai/v2/docs/)), then add the following to `~/.config/opencode/opencode.json`:

```json
{"mcp":{"servers":{"8ball":{"type":"remote","url":"http://127.0.0.1:8080/mcp","oauth":false,"headers":{"Authorization":"Bearer {env:EIGHTBALL_API_KEY}"}}}}}
```

```sh
ln -s "$PWD/plugin/skills/8-ball" ~/.agents/skills/8-ball
```

[Official MCP docs](https://opencode.ai/v2/docs/mcp-servers) show `mcp.servers` and `oauth: false` for bearer-only authentication. Start with the default Code Mode.

To test:

```sh
opencode mcp list
opencode run "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

Make sure that a decision tool call occurs and that the metric increases. If Code Mode cannot call the tool, record the failure first. Then try `"codemode": false` in that server entry.

To uninstall, remove only `mcp.servers.8ball` from the configuration and the skill symlink that you created.

## Development checks

Follow the [Rust style guide](docs/rust-style.md) for source and tests. Write commits in Commitizen style, and write docs in the Plain mode of [SimpleEnglish](https://github.com/AminBlg/SimpleEnglish). Do not loosen a live test assertion to make it pass.

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo nextest run
dev/models.sh up
cargo nextest run --test live --run-ignored only --no-capture
docker build -t 8ball:dev .
bash dev/image-smoke.sh 8ball:dev
claude plugin validate plugin --strict
```

This repository claims no public marketplace submission. A hosted privacy-policy URL for directory publication is deferred.

The live tests need `dev/models.sh up`. This script takes `up`, `down`, `status` (the default), or `logs [service]`. It runs `d3`, `embed`, `rerank`, and `chat` from `dev/compose.yaml` on ports 9931 to 9934. They use the same pinned llama.cpp image and models as the deployment. `dev/config.toml` is the matching configuration. `dev/image-smoke.sh <image>` tests the version, startup, SQLite initialization, authentication, and triage with a read-only root filesystem.

## Container image

The [container workflow](.github/workflows/image.yaml) builds the image, runs a smoke test, and publishes to GHCR. It uses `GITHUB_TOKEN` and needs no publishing token. The workflow builds for `linux/amd64`.

A push to `main` that changes code, the Docker build inputs, or the chart is a release. It moves `latest`, and gives the image, the chart and the Git tag the next patch version: `v0.1.0` first, then `v0.1.1`. The image is `ghcr.io/tanguille/8-ball:<version>`. The chart is `oci://ghcr.io/tanguille/charts/eightball`, with the version without the `v`. The GitHub release has generated notes.

A push that changes only docs is not a release. For a minor or major version, push the tag `vX.Y.0` on the last commit of `main`, then run `gh workflow run image.yaml --ref main`. That run releases the tag, and the next release continues from it. The same command finishes a release that failed halfway. To build another branch, run `gh workflow run image.yaml --ref <branch>`. That build publishes only the `sha-` image.

The image and chart packages are public, so that deployments and Renovate can read them without credentials. Keep that visibility.

### Helm chart

The [chart](charts/eightball) uses the bjw-s common library. Its values use the same format as app-template. The image tag defaults to the chart appVersion, which is the release tag. Install it with the release name `eightball`, so the Service is `eightball.<namespace>.svc`:

```sh
helm install eightball oci://ghcr.io/tanguille/charts/eightball --version 0.1.0
```

These options are off by default. Set them to `true` to turn them on:

- `serviceMonitor.app.enabled` scrapes `/metrics`.
- `rawResources.prometheusRule.enabled` adds the `EightballBackendErrors` alert.
- `configMapsFromFolder.enabled` adds the dashboard as the ConfigMap `<release>-dashboard`, with the label `grafana_dashboard: "1"` and the datasource input `DS_PROMETHEUS`.

The chart expects a ConfigMap `<release>-config` with a `config.toml` key, and it creates a 1 GiB PVC for the database. `helm uninstall` keeps that PVC. Set `persistence.data.existingClaim` to use your own PVC.

To use no ConfigMap, set `persistence.config.enabled: false`. Then, in `controllers.eightball.containers.app.env`, set `EIGHTBALL_CONFIG` to `null` and put the TOML in `EIGHTBALL_CONFIG_TOML`.

The image contains the `8ball` CLI and server, not models or server configuration. To run the image:

- Mount a TOML configuration read-only, or set `EIGHTBALL_CONFIG_TOML`.
- Provide writable storage for `EIGHTBALL_DB`.
- Set `EIGHTBALL_API_KEY`.
