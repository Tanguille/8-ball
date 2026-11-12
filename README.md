# 8-ball

> [!WARNING]
> 8-ball is in early development. Commands, APIs, configuration, and the database format can change without notice. Use it at your own risk.

**Ask a small model before you pay a big one.**

Agents spend seconds and tokens to read alerts, tickets, and files, often only to decide "not relevant". 8-ball gives that decision to a small local model. On the pinned dev models, an answer takes about 0.08 seconds. A repeated question comes from the cache in less than 0.01 seconds. If the model is not sure, it says `unsure`, and you do the normal work.

Our [benchmark](docs/benchmark.md) has 1,119 public cases. On the cases that it decides, the d3 decision model is as accurate as Qwen3.8 27B, and it is 15 times faster. If the 27B answers the cases where d3 is `unsure`, you get more right verdicts than with the 27B alone, 6.5 times faster.

## What you can do with it

The examples use the specs in [`dev/config.toml`](dev/config.toml). Start the server with `8ball serve --config dev/config.toml` before you run them.

### Skip the alerts that do not matter

Judge the alerts first. Your agent then reads only the alerts that 8-ball keeps:

```sh
echo '["Primary database is down, all writes fail", "Weekly report is ready", "Backups have failed for the last 4 nights", "New comment on your blog post"]' \
  | 8ball triage --spec storage-risk \
  | jq -c '{kept: [.kept[].index], rejected: [.rejected[].index], unsure: [.unsure[].index]}'
```

```json
{"kept":[0,2],"rejected":[1,3],"unsure":[]}
```

Why it pays: the big model reads 2 alerts, not 4.

### Route tickets without a big model

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

Why it pays: a local call replaces a paid call for each ticket.

### Search code by behavior

In [`examples/code-search.sh`](examples/code-search.sh), ripgrep finds the blocks that match a few keywords. 8-ball keeps the blocks that do the behavior.

```sh
examples/code-search.sh "reject a request with a wrong API key" 'api_key|bearer|401' service/src
```

```json
{"verdict":"match","file":"service/src/http.rs","lines":"160-184","confidence":0.99}
{"verdict":"check","file":"service/src/lib.rs","lines":"273-305","confidence":0.42}
```

Why it pays: the agent reads the matching blocks, not every file that has a keyword. You need no index. [More about the limits](docs/examples.md#search-code-by-behavior).

### Gate a script

```sh
if 8ball ask --spec storage-risk --state "$ALERT" --exit-code > /dev/null; then page_oncall; fi
```

Exit code 0 is `yes`, 1 is `no`, and 3 is `unsure`. See the [CLI](docs/cli.md).

### Get better with use

Label the outcome of the work. 8-ball then fits its thresholds to the precision that you ask for. It also remembers your labels for the same question and state. See [calibration and learning](docs/calibration.md).

## How it works

A question is `noul` (yes or no), `choice` (pick one option), or `score` (pick a level). The verdict is the answer. A low confidence gives `unsure`. Each answer goes into a SQLite cache.

8-ball is a Rust server and CLI. It uses System One decision models and rerankers on llama.cpp servers. It also uses chat models through logprobs on any OpenAI-compatible server that returns them, such as vLLM. Claude Code, Codex, Cursor, qwen-code, and OpenCode connect over MCP. Scripts use the CLI or REST.

The times come from `dev/models.sh up` with GPU offload, `8ball serve --config dev/config.toml`, and `/usr/bin/time` around `8ball ask`. Other models and hardware give other numbers.

## Quickstart

```sh
cargo install --path cli
dev/models.sh up        # four pinned dev models, about 4.5 GB
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

Then start the server, and ask in another terminal:

```sh
8ball serve
8ball ask --spec greeting --state "hey there"
```

For a shared server, set `EIGHTBALL_URL` and `EIGHTBALL_API_KEY`, then run `8ball doctor`. For CPU inference, use `EIGHTBALL_DEV_NGL=0 dev/models.sh up`.

## Docs

| Page | What it covers |
| --- | --- |
| [Examples](docs/examples.md) | JSON output, choice and score questions, triage, exit codes, code search |
| [Benchmark](docs/benchmark.md) | Accuracy and speed against a 27B model, and how to repeat it |
| [CLI](docs/cli.md) | Commands, flags, exit codes |
| [HTTP and MCP API](docs/http-api.md) | Routes, authentication, MCP tools |
| [Configuration](docs/configuration.md) | TOML file, environment variables, thinking chat models |
| [Calibration and learning](docs/calibration.md) | Labels, bands, and the learning loop |
| [MCP clients](docs/mcp-clients.md) | Claude Code, Codex, Cursor, qwen-code, OpenCode |
| [Container image and Helm chart](docs/deploy.md) | Run it on a server or in Kubernetes |
| [Privacy](docs/privacy.md) | What is stored and where requests go |
| [Hermes pilot](docs/hermes-triage.md) | GitOps digest pre-check and holdout replay |
| [Development](docs/development.md) | Checks, dev models, live tests |
