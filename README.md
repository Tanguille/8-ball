# 8-ball

> [!WARNING]
> 8-ball is in early development. Commands, APIs, configuration, and the database format can change without notice. Use it at your own risk.

8-ball helps AI agents decide what is worth their time. A small local model can pick which links to read, which alerts need attention, or where a support ticket belongs. If 8-ball returns `unsure`, the agent does the work as usual.

## Find the links worth reading

Say you ask an agent why your tomato plants are turning yellow. It finds 30 links and sends their titles and short descriptions to 8-ball. It asks, “Does this page explain why tomato leaves turn yellow?” The agent reads the pages that 8-ball keeps or marks `unsure`.

Once you [connect 8-ball to your agent](docs/mcp-clients.md) through the plugin or MCP, the agent can ask questions on the spot. It needs no script or saved question. Reading fewer pages saves tokens and leaves more room for useful information during long tasks.

## What you can do with it

Install `8ball` as shown in the [quickstart](#quickstart). Run `dev/models.sh up` to start the models for these examples. The first run downloads about 4.5 GB. Then run `8ball serve --config dev/config.toml` to start 8-ball with the [example configuration](dev/config.toml).

### Skip the alerts that do not matter

A spec is a question saved in the configuration. The `storage-risk` spec helps an agent find the alerts that need attention.

```sh
echo '["Primary database is down, all writes fail", "Weekly report is ready", "Backups have failed for the last 4 nights", "New comment on your blog post"]' \
  | 8ball triage --spec storage-risk \
  | jq -c '{kept: [.kept[].index], rejected: [.rejected[].index], unsure: [.unsure[].index]}'
```

```json
{"kept":[0,2],"rejected":[1,3],"unsure":[]}
```

8-ball keeps two alerts for the agent to read.

### Route tickets without a big model

Ask 8-ball to choose a team based on the text of a support ticket.

```sh
8ball ask choice "Which team should handle this ticket?" \
  --option billing="Invoices, payments and refunds" \
  --option shipping="Parcels and delivery" \
  --option security="Accounts and access" \
  --state "I was charged twice for my subscription this month." \
  | jq -c '.results[0] | {verdict, confidence}'
```

```json
{"verdict":"billing","confidence":0.99}
```

8-ball returns `billing` so that the agent knows where to send the ticket.

### Search code by behavior

The [code search script](examples/code-search.sh) uses ripgrep to find code that matches your keywords. 8-ball then judges whether each block does what you asked for.

```sh
examples/code-search.sh "reject a request with a wrong API key" 'api_key|bearer|401' service/src
```

```json
{"verdict":"match","file":"service/src/http.rs","lines":"4-58","confidence":0.96}
{"verdict":"match","file":"service/src/http.rs","lines":"128-182","confidence":0.98}
{"verdict":"check","file":"service/src/lib.rs","lines":"370-402","confidence":0.02}
```

The agent can start with the blocks marked `match` and read the ones marked `check` when 8-ball is unsure. The script reads the files on disk, so it works without a search index. See the [code search limits](docs/examples.md#search-code-by-behavior).

### Gate a script

Use the answer to decide whether a script pages the on-call person.

```sh
if 8ball ask --spec storage-risk --state "$ALERT" --exit-code > /dev/null; then page_oncall; fi
```

With `--exit-code`, 0 means `yes`, 1 means `no`, and 3 means `unsure`. A missing spec or a server error returns 4. This example pages only on 0. If a missed page costs more than a false one, handle code 4 too. See the [CLI reference](docs/cli.md).

### Get better with use

After you do the work, give 8-ball the answer that the evidence supports. 8-ball can remember that answer for the same question and input. It can also use labels to adjust when it says `unsure`. See [calibration and learning](docs/calibration.md).

## How it works

A question can ask for yes or no (`noul`), one option (`choice`), or a level (`score`). The verdict is the answer that 8-ball returns. When confidence is too low, the verdict is `unsure`. 8-ball saves each answer in SQLite for reuse.

8-ball is a Rust server with a command-line tool. Claude Code, Codex, Cursor, qwen-code, and OpenCode connect over MCP. Scripts use the command-line tool or the HTTP API.

8-ball supports System One decision models and rerankers on llama.cpp. A reranker scores how well an input matches the topic of a question. 8-ball also supports chat models on OpenAI-compatible servers such as vLLM if they return logprobs, which measure the likelihood of each answer token.

On the development models, an answer takes about 0.08 seconds. A repeat answer takes less than 0.01 seconds. These times use GPU offload with `dev/models.sh up` and `8ball serve --config dev/config.toml`. We measured `8ball ask` with `/usr/bin/time`. Times vary with the model and hardware.

In our [benchmark](docs/benchmark.md) of 1,119 public cases, d3 was about as accurate as Qwen3.8 27B on the cases it decided. It answered 15 times as fast. Using the 27B model for cases where d3 returned `unsure` got more answers right than the 27B alone and ran 6.5 times as fast.

## Quickstart

From the repository root, install `8ball` and start the development models.

```sh
cargo install --path cli
dev/models.sh up        # four pinned dev models, about 4.5 GB
mkdir -p ~/.config/8ball
```

Create `~/.config/8ball/config.toml` with this content.

```toml
default = "d3"

[endpoints.d3]
url = "http://127.0.0.1:9931"

[specs.greeting]
type = "noul"
instructions = "Is this message a greeting?"
```

Start the server.

```sh
8ball serve
```

Ask a question from a second terminal.

```sh
8ball ask --spec greeting --state "hey there"
```

For a shared server, set `EIGHTBALL_URL` and `EIGHTBALL_API_KEY`. Then run `8ball doctor`. To run the models on a CPU, use `EIGHTBALL_DEV_NGL=0 dev/models.sh up`.

## Docs

| Page | What it covers |
| --- | --- |
| [Examples](docs/examples.md) | JSON output, choice and score questions, exit codes, code search |
| [Benchmark](docs/benchmark.md) | Accuracy and speed against a 27B model, how to repeat it, and the CI benchmark runs |
| [CLI](docs/cli.md) | Commands, flags, exit codes |
| [HTTP and MCP API](docs/http-api.md) | Routes, authentication, MCP tools |
| [Configuration](docs/configuration.md) | TOML file, environment variables, thinking chat models |
| [Calibration and learning](docs/calibration.md) | Use known answers to improve decisions |
| [MCP clients](docs/mcp-clients.md) | Claude Code, Codex, Cursor, qwen-code, OpenCode |
| [Container image and Helm chart](docs/deploy.md) | Run it on a server or in Kubernetes |
| [Privacy](docs/privacy.md) | What is stored and where requests go |
| [Hermes pilot](docs/hermes-triage.md) | Test which GitOps updates are worth a closer look |
| [Development](docs/development.md) | Checks, dev models, live tests |
