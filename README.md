# 8-ball

> [!WARNING]
> 8-ball is in early development. Commands, APIs, configuration, and the database format can change without notice. Use it at your own risk.

8-ball uses a small model to help AI agents decide what is worth reading or acting on. If 8-ball returns `unsure`, the agent does the work as usual.

## Find the links worth reading

Say you ask an agent why your tomato plants are turning yellow. It finds 30 links and sends the titles and short descriptions to 8-ball. It asks which pages explain yellow leaves. The agent reads the links that 8-ball keeps or marks `unsure`.

Once you [connect the plugin or MCP](docs/mcp-clients.md), the agent can ask questions like this without a script or a saved question. Reading fewer full pages saves tokens and leaves room for useful information during long tasks.

## What you can do with it

### Skip the alerts that do not matter

A spec is a saved question. The `storage-risk` spec picks out alerts about data or storage problems. Start 8-ball as shown in the [quickstart](#quickstart), then run this command in another terminal.

```sh
echo '["Primary database is down, all writes fail", "Weekly report is ready", "Backups have failed for the last 4 nights", "New comment on your blog post"]' \
  | 8ball triage --spec storage-risk \
  | jq -c '{kept: [.kept[].index], rejected: [.rejected[].index], unsure: [.unsure[].index]}'
```

```json
{"kept":[0,2],"rejected":[1,3],"unsure":[]}
```

### Route tickets without a big model

Ask 8-ball to [pick a team from the ticket text](docs/examples.md#ask-a-question-on-the-spot).

### Search code by behavior

Use ripgrep to find candidate snippets, then ask 8-ball [which ones do what you are looking for](docs/examples.md#search-code-by-behavior).

### Gate a script

Use a `yes` answer to trigger an action.

```sh
if 8ball ask --spec storage-risk --state "$ALERT" --exit-code > /dev/null; then page_oncall; fi
```

This example pages only on exit code 0. If you need to handle errors separately, see the [CLI exit codes](docs/cli.md).

### Get better with use

Give 8-ball [known answers](docs/calibration.md) so it can learn when to decide and when to say `unsure`.

## How it works

Ask for yes or no (`noul`), one option (`choice`), or a level (`score`). 8-ball saves answers in SQLite for reuse.

With the development models on a GPU, `8ball ask` took about 80 ms, or under 10 ms for a cached answer. In our [benchmark](docs/benchmark.md), d3 with Qwen3.8 27B for `unsure` cases got more answers right than 27B alone. It ran 6.5 times as fast.

## Quickstart

Install `8ball` and start the development models from the repository root. The first run downloads about 4.3 GB. To use a CPU, run `EIGHTBALL_DEV_NGL=0 dev/models.sh up`.

```sh
cargo install --path cli
dev/models.sh up
8ball serve --config dev/config.toml
```

Ask a saved question in a second terminal.

```sh
8ball ask --spec greeting --state "hey there"
```

If you use a shared server, set `EIGHTBALL_URL` and `EIGHTBALL_API_KEY`. Then run `8ball doctor`.

## Docs

| Page | What it covers |
| --- | --- |
| [Examples](docs/examples.md) | JSON output, choice and score questions, exit codes, code search |
| [Benchmark](docs/benchmark.md) | Accuracy, speed, and how to run the same tests |
| [CLI](docs/cli.md) | Commands, flags, exit codes |
| [HTTP and MCP API](docs/http-api.md) | Routes, authentication, MCP tools |
| [Configuration](docs/configuration.md) | Model servers, saved questions, and environment variables |
| [Calibration and learning](docs/calibration.md) | Use known answers to improve decisions |
| [MCP clients](docs/mcp-clients.md) | Claude Code, Codex, Cursor, qwen-code, OpenCode |
| [Container image and Helm chart](docs/deploy.md) | Run it on a server or in Kubernetes |
| [Privacy](docs/privacy.md) | What is stored and where requests go |
| [Hermes pilot](docs/hermes-triage.md) | Test which GitOps updates are worth a closer look |
| [Development](docs/development.md) | Checks, dev models, live tests |
