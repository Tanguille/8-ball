# Examples

Run `dev/models.sh up` to start the development models. Then run `8ball serve --config dev/config.toml` to start 8-ball.

We took these results from the first run with an empty database. The run used the default decision thresholds, with no calibration. Results can vary between runs and with different models, hardware, or versions.

## Ask a saved question

A spec is a question saved in the configuration. Use `--state` to pass the text that 8-ball judges. The command prints one line of JSON. The pipe to `jq` formats it for reading.

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
      "confidence": 0.9945411770492965,
      "probabilities": {
        "yes": 0.9972705885246482,
        "no": 0.002729411475351773
      },
      "calibrated": false,
      "cached": false,
      "report": true,
      "explored": false,
      "remembered": false,
      "escalated": false
    }
  ]
}
```

`confidence` shows how sure the model is. The decision thresholds determine which answer 8-ball returns. If confidence is too low, 8-ball returns `unsure`. Do the work as usual in that case.

## Ask a question on the spot

Use `choice` to pick one option or `score` to pick one level. Define the question and its options in the command.

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

A score verdict is the position of the chosen level, starting at 0 for the lowest level. The table rounds the confidence scores.

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

## Filter a list

Use `triage` to judge a list of items with the same question. An index in the output is the item's original position, starting at 0. See the [alert example](../README.md#skip-the-alerts-that-do-not-matter).

## Use the answer in a shell script

Pass `--exit-code` to let a script tell `yes`, `no`, and `unsure` apart by their exit codes.

```sh
8ball ask --spec greeting --state "hey there, how are you?" --exit-code > /dev/null; echo $?                    # 0: yes
8ball ask --spec greeting --state "Please send me the invoice for October." --exit-code > /dev/null; echo $?    # 1: no
8ball ask noul "Does this alert need a human now?" --state "Weekly report is ready" --exit-code > /dev/null; echo $?   # 3: unsure
```

Agents can ask questions with `decision_ask` or filter lists with `decision_triage` over MCP.

## Search code by behavior

The [code search script](../examples/code-search.sh) uses ripgrep to find blocks that match a few keywords. 8-ball judges each block against your question. The script searches the files on disk, so it includes uncommitted edits and works without an index.

```sh
examples/code-search.sh "reject a request with a wrong API key" 'api_key|bearer|401' service/src
```

```json
{"verdict":"match","file":"service/src/http.rs","lines":"4-58","confidence":0.96}
{"verdict":"match","file":"service/src/http.rs","lines":"128-182","confidence":0.98}
{"verdict":"check","file":"service/src/lib.rs","lines":"370-402","confidence":0.02}
```

Lines 128 to 182 check the bearer key. Read blocks marked `check` yourself because 8-ball returned `unsure` for those blocks. The script prints `3 blocks: 2 match, 1 check, 0 no` to standard error. This run took 0.9 seconds with an empty database.

After the pattern, you can add paths and ripgrep flags such as `service/src cli/src -g '!**/tests/**'`. The default path is the current directory. The script uses `8ball triage --no-learn` so that exploration cannot turn a `no` into a `check`.

### Limits

- 8-ball sees only the blocks that ripgrep finds. For "remove the middle of a state that is too long", the pattern `max_state_chars|trim` in `backend/src/lib.rs` found only the caller. 8-ball marked it `check` (0.6). The `fit` function removes the middle. It has neither keyword. The pattern `'fn fit'` found it, and 8-ball marked it `match` (0.97).
- `8ball triage` sends groups of 64 blocks in order. More blocks take longer to judge. Use more specific keywords or a narrower path to speed up the search.
- The script takes 12 lines before and after each keyword match. That window can show only part of a long function.
