# CLI

A spec is a question saved in the configuration. The state is the text that 8-ball judges. An endpoint is a model server listed in the configuration. Its `kind` sets how 8-ball talks to it.

Use `noul` for yes or no, `choice` to pick one option, and `score` to pick a level. The lowest score level has position 0.

Start an 8-ball server before you run client commands. The client refuses HTTP redirects. The commands `serve`, `mcp`, and `config check` run on their own.

| Command | Purpose |
| --- | --- |
| `serve` | Start the REST and MCP server over HTTP. |
| `mcp` | Start an MCP server that reads standard input and writes to standard output. |
| `ask [<kind> <instructions>]` | Pass either a new question or `--spec` for a saved one. Pass the text with `--state` or through standard input. Use `--exit-code` to enable codes 1 and 3. |
| `triage [<instructions>]` | Read a JSON array from standard input. Give either one new question or one `--spec`. Set the question type with `--type` (default `noul`). Use `--context-file` to pass shared evidence from a UTF-8 file. If you configure `[embedder]`, use `--top-k` to judge only the items most similar to the question. See [larger lists](#larger-lists). |
| `observe` | Save decisions in a private file. The JSON output includes `wakeAgent: true` and every candidate. Pass `--spec`, `--input`, and `--observations`. The input is JSON with `context` text and up to 64 `items`. The observations path must be new. 8-ball writes one JSONL record there. |
| `evaluate` | Read labeled JSONL cases from standard input and run them against a required `--spec`. Report misses, `unsure` verdicts, errors, and response times. Keep the current calibration. |
| `questions` | List stored questions with counts for decisions, `unsure` answers, and labels. Use the list to find repeated questions worth saving as specs. |
| `decisions --spec <spec>` | Show the current spec version's verdicts and input text, newest first. Use `--unlabelled` to hide labeled rows. Use `--question <hash>` for a one-off question. To view an older spec version, pass its hash with `--spec`. |
| `label <id> <value>` | Save the known answer for a decision. Use `yes`, `no`, an option name, or the position of a score level. This works for current specs and, with `learning.ad_hoc` on, for one-off questions. |
| `calibrate <spec>` | Set decision thresholds from known answers. `--precision` accepts 0.5 to 0.99 and defaults to 0.95. Use `--dry-run` to see the report without saving. |
| `doctor` | Show how 8-ball detected each endpoint, whether it responds, the model fingerprint, and the calibration for each spec. |
| `config check [<path>]` | Make sure that a TOML file is valid. If you omit `<path>`, 8-ball uses the [default configuration path](configuration.md), even if you set `EIGHTBALL_CONFIG`. |

| Shared flag | Commands | Purpose |
| --- | --- | --- |
| `--endpoint <name>` | `ask`, `triage`, `evaluate`, `observe` | Override the endpoint. |
| `--option <name[=description]>`, `--level <text>` | `ask`, `triage` | Add a choice option or a score level. You can repeat either flag. Give score levels in order from lowest to highest. |
| `--keep <value>` | `triage`, `evaluate` | For `noul`, keep `yes` by default or set `no`. For `choice` or `score`, set an option name or a minimum level index. |
| `--no-learn` | `ask`, `triage` | Turn off remembered labels, exploration, and reports. This sends `"learn": false`. |
| `--limit <n>` | `questions`, `decisions` | List up to this many rows. Set a number from 1 to 500. The default is 50. |
| `--timeout-ms <ms>` | All that call a server | Set the request deadline in milliseconds, with a minimum of 1. The default is 25,000 for `observe` and `evaluate`. Other commands have no deadline by default. |

[Environment variables](configuration.md#environment-variables) lists the other flags. `8ball --version` prints the version.

| Exit code | Meaning |
| --- | --- |
| 0 | Success. With `--exit-code`, the verdict is `yes`, an option, or a level. |
| 1 | With `--exit-code`, the noul verdict is `no`. |
| 2 | The command rejected the arguments. |
| 3 | With `--exit-code`, the verdict is `unsure`. |
| 4 | Any other error. For example, `ask` needs `--spec` or `<kind> <instructions>`. For `doctor`, the default endpoint failed. For `evaluate`, a case failed. |

`observe` reports input, model, and write errors in its JSON and still exits with code 0.

## Larger lists

For arrays with more than 64 items, `triage` sends groups of 64 in order. The output keeps each item's original position. The `--top-k` filter and identical-item matching apply within each group. If a request fails, `triage` stops without printing a result and exits with code 4. On a retry, it can reuse saved answers from the groups that finished.

## Read an answer

Use `results[].id` to label the saved decision. The `results[].question` field holds the question id that the caller sent. The `results[].type` field holds the question type. When you repeat the same request for a spec, 8-ball reuses its row. Each spec and each one-off question has a separate set of rows. If the model server returns invalid probabilities, 8-ball returns a backend error.

A result can include these fields.

- `report: true` asks you to do the work. Then label the id with what you found.
- `explored: true` means that 8-ball wants you to look at an item the model would skip (`noul` `no` or triage `rejected`). The verdict is `unsure`.
- `remembered: true` means that 8-ball used a label from an earlier answer to the same question and text.

For `/v1/ask` and `/v1/triage`, pass `"learn": false` to turn these three features off for one request. The default is `true`.
