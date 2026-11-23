# HTTP and MCP API

If the server has `EIGHTBALL_API_KEY` set, send `Authorization: Bearer <key>` to use a protected route. The server returns 401 if the key is missing or wrong.

Without a key, `/mcp` and `/v1` accept only the hosts `localhost`, `127.0.0.1`, and `::1`. Other hosts get 403. This restriction stops a web page from reaching the local server through DNS rebinding. To serve other hosts, set a key.

| Route | Protected | Purpose |
| --- | --- | --- |
| `POST /v1/ask`, `POST /v1/triage` | Yes | Accept the same input as `decision_ask` and `decision_triage`, respectively. |
| `POST /v1/label`, `POST /v1/calibrate` | Yes | Accept the CLI arguments as JSON. Use `id` and `value` for a label. Use `spec`, `precision`, and `dry_run` for calibration. |
| `POST /v1/questions`, `POST /v1/decisions` | Yes | Accept the CLI arguments as JSON, including `limit`, `spec`, `question`, and `unlabelled` where applicable. Set `limit` from 1 to 500. The default is 50. Every `/v1/decisions` row includes the `endpoint` that answered. |
| `GET /v1/doctor` | Yes | Return the `doctor` report. |
| `/mcp` | Yes | Serve HTTP MCP. |
| `GET /healthz` | No | Return `ok`. |
| `GET /metrics` | No | Return OpenMetrics. Counters include `eightball_decisions_total` (with `cached`), `eightball_backend_errors_total` (including missed deadlines), and `eightball_trimmed_states_total`. The `eightball_escalations_total` counter has `endpoint` and `outcome` labels. Its outcomes are `decided`, `unsure`, `error`, and `budget`. The histogram is `eightball_backend_request_duration_seconds`. |
| `POST /register`, `GET /.well-known/{*rest}` | No | Return 404, because 8-ball does not use OAuth. |

The server returns a JSON `error` field for service, authentication, and host errors. If it rejects the request body, it returns plain text.

## MCP tools

The state is the input text. A spec is a saved question.

| Tool | Purpose |
| --- | --- |
| `decision_ask` | Ask questions about `state`. Pass `spec` for a saved question or `questions` to define your own. The `endpoint`, `learn`, and `escalate` fields are optional. |
| `decision_triage` | Judge up to 64 `items` with one `spec` or `question`. The `context`, `keep`, `top_k`, `endpoint`, `learn`, and `escalate` fields are optional. Each index appears once in `kept`, `unsure`, `rejected`, or `prefiltered`. The `prefiltered` group holds the items that `top_k` left out. Identical items share one decision. |
| `decision_questions` | List saved questions with counts for decisions, `unsure` answers, and labels. |
| `decision_review` | List unlabeled decisions for a `spec`, with their ids and input text. The tool hides the model's answer so that you can label the decision independently. |
| `decision_label` | Save a label `value` for a decision `id`, as with the CLI `label` command. After you do the work, call this for each decision with `report: true`. |

## Decisions

A decision has `escalated: true` when its verdict came from the [escalation endpoint](configuration.md#escalation), after the default endpoint was `unsure`. Its `id` is then the escalation's stored row. `escalate: false` on a request keeps the `unsure` answer.

A single `ask` has a 20 second deadline, escalation included; a primary answer that misses it fails the request. A triage has a 90 second deadline, prefilter and escalation included. An escalation that misses its deadline keeps the `unsure` answer. A triage item that failed (backend error, store error, or the deadline) comes back in `unsure` with an `error` field and `id: 0`. 8-ball does not store or learn from it. The response `warning` then says `<n> of <total> items failed and are unsure: <first error>`. If every item failed, the request fails with the first error.
