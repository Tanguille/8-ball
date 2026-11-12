# CLI

A spec is a named question in the configuration. The state is the text to judge. An endpoint is a configured model server. Its `kind` is the backend, which sets how 8-ball talks to that server. All commands except `serve`, `mcp`, and `config check` call a running server and refuse HTTP redirects.

| Command | Purpose |
| --- | --- |
| `serve` | Run REST and HTTP MCP. |
| `mcp` | Serve MCP over stdin and stdout. |
| `ask [<kind> <instructions>]` | Ask one ad-hoc question or a `--spec`. `--state` gives the state. The default is stdin. `--exit-code` enables exit codes 1 and 3. |
| `triage [<instructions>]` | Judge a JSON array from stdin with one question or a `--spec`, not both. `--type` sets the question type (default `noul`). `--context-file` names a UTF-8 file with shared evidence for every item. `--top-k` judges only the items most similar to the question and needs `[embedder]`. |
| `observe` | Record the triage decisions of a snapshot in a private file. Print JSON with `wakeAgent: true` and every candidate. It requires `--spec`, `--input` (JSON with `context` text and at most 64 `items`), and `--observations` (a new JSONL file, never overwritten). |
| `evaluate` | Replay labeled JSONL cases from stdin against a required `--spec`. Report misses, `unsure` verdicts, errors, and latency. Calibration does not change. |
| `questions` | List the stored questions with their decision, `unsure`, and label counts. Find ad-hoc questions that come back often, and save them as specs. |
| `decisions --spec <spec>` | List the stored decisions of the current spec version, newest first, with verdicts and states. `--unlabelled` hides labeled ones. `--question <hash>` lists an ad-hoc question, or an older spec version together with `--spec`. |
| `label <id> <value>` | Label a saved decision of a current spec, or of an ad-hoc question when `learning.ad_hoc` is on. The value is `yes`, `no`, an option name, or a level index. |
| `calibrate <spec>` | Fit bands (decision thresholds) from labeled states. `--precision` takes 0.5 to 0.99 and defaults to 0.95. `--dry-run` reports without a save. |
| `doctor` | Report endpoint detection, health, model fingerprints, and spec calibration. |
| `config check [<path>]` | Make sure that a TOML file is valid, without a server. `<path>` defaults to the [configuration path](configuration.md), not `EIGHTBALL_CONFIG`. |

| Shared flag | Commands | Purpose |
| --- | --- | --- |
| `--endpoint <name>` | `ask`, `triage`, `evaluate`, `observe` | Override the endpoint. |
| `--option <name[=description]>`, `--level <text>` | `ask`, `triage` | Add a choice option or a score level, lowest first. Both repeat. |
| `--keep <value>` | `triage`, `evaluate` | Keep `yes` (default), `no`, an option name, or a minimum level index. The default works for noul only, so choice and score questions need `--keep`. |
| `--timeout-ms <ms>` | All that call a server | Total request deadline, at least 1. The default is 25,000 for `observe` and `evaluate`, otherwise none. |

[Environment variables](configuration.md#environment-variables) lists the other flags. `8ball --version` prints the version.

| Exit code | Meaning |
| --- | --- |
| 0 | Success. With `--exit-code`, the verdict is `yes`, an option, or a level. |
| 1 | With `--exit-code`, the noul verdict is `no`. |
| 2 | The argument parser rejected the arguments. |
| 3 | With `--exit-code`, the verdict is `unsure`. |
| 4 | Any other error, for example `ask` with neither `--spec` nor `<kind> <instructions>`. For `doctor`, the default endpoint failed. For `evaluate`, a case failed. |

`observe` exits 0 after input, model, and write errors. Its JSON reports them.

`results[].id` is the saved row id for `label`. `results[].question` is the question id of the caller. `results[].type` is the question type. An identical request within a spec reuses a row. Ad-hoc questions and different specs have separate rows. Invalid backend probabilities return a backend error.

A result can also have `report`, `explored`, and `remembered`. `report: true` asks the caller to do the work, then label the id with what the work showed. `explored: true` means that the model skipped the item (noul `no`, triage `rejected`), but 8-ball returns `unsure` so that the caller checks it. `remembered: true` means that the verdict is a label reported earlier for the same question and state. `/v1/ask` and `/v1/triage` take `"learn": false` (default `true`) to turn all 3 off for one request.

