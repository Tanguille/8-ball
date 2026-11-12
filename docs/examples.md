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

## Search code by behavior

[`examples/code-search.sh`](../examples/code-search.sh) finds code by what it does. ripgrep finds blocks that match a few keywords. 8-ball then reads each block and keeps those that do the behavior. You need no index, so uncommitted edits are included.

```sh
examples/code-search.sh "reject a request with a wrong API key" 'api_key|bearer|401' service/src
```

```json
{"verdict":"match","file":"service/src/http.rs","lines":"4-58","confidence":0.96}
{"verdict":"match","file":"service/src/http.rs","lines":"160-184","confidence":0.99}
{"verdict":"check","file":"service/src/http.rs","lines":"128-152","confidence":0.06}
{"verdict":"check","file":"service/src/lib.rs","lines":"273-305","confidence":0.42}
```

Lines 160 to 184 hold the bearer key check. `check` means `unsure`, so read those blocks yourself. This run took 0.9 seconds with an empty database.

Limits:

- 8-ball judges only the blocks that ripgrep finds. If the keywords miss the code, 8-ball misses it too. For "remove the middle of a state that is too long", `max_state_chars|trim` in `backend/src/lib.rs` found only the caller, as `check` (0.67). The `fit` function cuts the state, but has neither keyword. With `'fn fit'`, it is a `match` (0.97).
- `triage` takes at most 64 blocks. If ripgrep finds more, the script stops with an error, so use narrow keywords or a path.
- A block is the match with 12 lines before and after it, not a full function. The model can see only part of a long function.
