# HTTP and MCP API

If `EIGHTBALL_API_KEY` is set on the server, the protected routes need `Authorization: Bearer <key>`. A missing or wrong key returns 401. Without a key, `/mcp` and `/v1` accept only the hosts `localhost`, `127.0.0.1`, and `::1`, and other hosts get 403. This check stops a web page from reaching a local server through DNS rebinding. To serve other hosts, set a key.

| Route | Protected | Purpose |
| --- | --- | --- |
| `POST /v1/ask`, `POST /v1/triage` | Yes | Take the input of `decision_ask` and `decision_triage`. |
| `POST /v1/label`, `POST /v1/calibrate` | Yes | Take the CLI arguments as JSON: `id`, `value`, `spec`, `precision`, `dry_run`. |
| `POST /v1/questions`, `POST /v1/decisions` | Yes | Take the CLI arguments as JSON: `limit`, `spec`, `question`, `unlabelled`. `limit` is 1 to 500, and 50 by default. Each `/v1/decisions` row has the `endpoint` that answered. |
| `GET /v1/doctor` | Yes | Return the `doctor` report. |
| `/mcp` | Yes | Serve HTTP MCP. |
| `GET /healthz` | No | Return `ok`. |
| `GET /metrics` | No | Return OpenMetrics: `eightball_decisions_total` (with a `cached` label), `eightball_backend_errors_total` (missed request deadlines included), `eightball_trimmed_states_total`, `eightball_escalations_total` (with `endpoint` and `outcome`: `decided`, `unsure`, `error`, `budget`), and the histogram `eightball_backend_request_duration_seconds`. |
| `POST /register`, `GET /.well-known/{*rest}` | No | Return 404, because 8-ball does not use OAuth. |

Service, authentication, and host errors are JSON with an `error` field. Rejected request bodies return plain text.

## MCP tools

| Tool | Purpose |
| --- | --- |
| `decision_ask` | Ask typed questions about `state`. It takes `spec` or `questions`, and an optional `endpoint`, `learn`, and `escalate`. |
| `decision_triage` | Judge at most 64 `items` with one `spec` or `question`. It also takes optional `context`, `keep`, `top_k`, `endpoint`, `learn`, and `escalate`. Every index is in one list, and identical items share one decision. |
| `decision_questions` | List the stored questions with their decision, `unsure`, and label counts. |
| `decision_review` | List the unlabeled decisions of a `spec` with their ids and states. It leaves out the model's answer, so a label stays independent. |
| `decision_label` | Label a decision `id` with a `value`, as `label` does. Agents call it for each decision with `report: true`, after the work. |

## Decisions

A decision has `escalated: true` when its verdict came from the [escalation endpoint](configuration.md#escalation), after the default endpoint was `unsure`. Its `id` is then the escalation's stored row. `escalate: false` on a request keeps the `unsure` answer.

A single `ask` has a 20 second deadline, escalation included; a primary answer that misses it fails the request. A triage has a 90 second deadline, prefilter and escalation included. An escalation that misses its deadline keeps the `unsure` answer. A triage item that failed (backend error, store error, or the deadline) comes back in `unsure` with an `error` field and `id: 0`. 8-ball does not store or learn from it. The response `warning` then says `<n> of <total> items failed and are unsure: <first error>`. If every item failed, the request fails with the first error.
