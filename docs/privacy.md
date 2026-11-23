# Privacy

## What 8-ball stores

8-ball saves the full input and question text for every successful decision. It uses file mode 0600, which allows only the file owner to read and write the database. Use `retention_days` to remove old decisions. 8-ball keeps labels and all answers to the same question and state as a label. It deletes a question's text when no decision uses it.

## Where requests go

8-ball sends model requests to the servers in your configuration. When you use an MCP client, that client's model provider also receives your prompts.

Before you send private data to a shared server, read the policy of its operator.

## Database upgrades and backups

8-ball uses SQLx with bundled SQLite. You need no separate database server or SQLx CLI. SQLite is the only supported database.

At startup, 8-ball updates the database schema. It keeps decisions, labels, calibrations, and id counters.

Before an upgrade, back up the database with a tool that supports SQLite. The backup includes committed data from SQLite's write-ahead log (WAL). Keep backups outside the repository. See the [Helm chart](deploy.md#helm-chart) for Kubernetes storage.
