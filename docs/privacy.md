# Privacy

Storage uses SQLx with bundled SQLite, so you need no database server or SQLx CLI. Startup applies embedded migrations and keeps decisions, labels, calibrations, and id counters, including from the original v1 database. Before an upgrade, back up an existing database with a SQLite-aware tool. This backup includes committed data in the WAL (the SQLite write-ahead log). SQLite is the only supported database. PostgreSQL and persistent vector search are future work.

The database stores the raw text of every successful decision and of its question, with file mode 0600. `retention_days` never deletes labeled rows. It deletes a question's text when no decision uses that question any more. The [Helm chart](deploy.md#helm-chart) is in this repository. Backups are outside it. Model requests go to your configured endpoints. Prompts in an MCP client also go to the model provider of that client.

Before you send private data to a shared server, read the policy of its operator.
