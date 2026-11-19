# Examples

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

To triage a list with one spec, see the [README](../README.md#skip-the-alerts-that-do-not-matter).

Gate a shell script with `--exit-code`:

```sh
8ball ask --spec greeting --state "hey there, how are you?" --exit-code > /dev/null; echo $?                    # 0: yes
8ball ask --spec greeting --state "Please send me the invoice for October." --exit-code > /dev/null; echo $?    # 1: no
8ball ask noul "Does this alert need a human now?" --state "Weekly report is ready" --exit-code > /dev/null; echo $?   # 3: unsure
```

Agents get the same verdicts through the MCP tools `decision_ask` and `decision_triage`.

## Search code by behavior

[`examples/code-search.sh`](../examples/code-search.sh) finds code by what it does. ripgrep finds blocks that match a few keywords. 8-ball then reads each block and keeps those that do the behavior. You need no index, so uncommitted edits are included.

```sh
examples/code-search.sh "reject a request with a wrong API key" 'api_key|bearer|401' service/src
```

```json
{"verdict":"match","file":"service/src/http.rs","lines":"4-58","confidence":0.96}
{"verdict":"match","file":"service/src/http.rs","lines":"128-182","confidence":0.98}
{"verdict":"check","file":"service/src/lib.rs","lines":"370-402","confidence":0.02}
```

Lines 128 to 182 hold the bearer key check. `check` means `unsure`, so read those blocks yourself. On stderr, the script prints `3 blocks: 2 match, 1 check, 0 no`. This run took 0.9 seconds with an empty database.

After the pattern, you can give more paths and ripgrep flags, for example `service/src cli/src -g '!**/tests/**'`. The default is the current directory. The script uses `8ball triage --no-learn`, so exploration never turns a `no` into `check`.

Limits:

- 8-ball judges only the blocks that ripgrep finds. If the keywords miss the code, 8-ball misses it too. For "remove the middle of a state that is too long", `max_state_chars|trim` in `backend/src/lib.rs` found the caller, as `check` (0.6), but not `fit`, the function that cuts the state: it has neither keyword. With `'fn fit'`, `fit` is a `match` (0.97).
- `8ball triage` sends 64 blocks per request, one request after another. So many blocks take longer, and narrow keywords or paths make the search faster.
- A block is the match with 12 lines before and after it, not a full function. The model can see only part of a long function.
