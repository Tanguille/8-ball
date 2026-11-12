# MCP clients

Run the commands on this page from the repository root. `EIGHTBALL_URL` configures the CLI and the Claude plugin. The other clients need the URL edits in their sections.

## Claude Code

To install:

```sh
claude plugin marketplace add .
claude plugin install 8-ball@8-ball
```

The plugin also uses `EIGHTBALL_API_KEY`. A local server with no key accepts the empty bearer header.

To test:

```sh
claude -p --plugin-dir ./plugin \
  --allowedTools 'mcp__plugin_8-ball_8ball__decision_ask,mcp__plugin_8-ball_8ball__decision_triage' \
  "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
curl -s http://127.0.0.1:8080/metrics
```

We tested this on 2026-11-03 with Claude Code 2.1.317. The plugin tool call reported a verdict and increased the decision counter. Strict standalone `--mcp-config plugin/.mcp.json` uses the `mcp__8ball__` prefix instead.

To uninstall:

```sh
claude plugin uninstall 8-ball@8-ball
```

## Codex

To install:

```sh
codex plugin marketplace add .
codex plugin add 8-ball@8-ball
```

For a shared server, edit the URL in `plugin/.mcp.codex.json` before installation, because Codex does not expand environment variables in URLs. The key comes from `EIGHTBALL_API_KEY`.

Fallback:

```sh
codex mcp add 8ball --url http://127.0.0.1:8080/mcp --bearer-token-env-var EIGHTBALL_API_KEY
ln -s "$PWD/plugin/skills/8-ball" ~/.agents/skills/8-ball
```

To test:

```sh
codex exec --ephemeral \
  -c 'mcp_servers.8ball.url="http://127.0.0.1:8080/mcp"' \
  -c 'mcp_servers.8ball.tools.decision_ask.approval_mode="approve"' \
  'Call decision_ask with {"state":"disk 98% full on /var","questions":[{"id":"urgent","type":"noul","instructions":"Is this alert urgent?"}]}, then report the verdict.'
```

The one-tool approval override applies only to this synthetic local test. Normal installed sessions can approve calls interactively. [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) documents approval for each tool.

We tested this on 2026-11-03 with Codex CLI 0.166.0 and the per-run MCP fallback, not a persistent marketplace installation. The HTTP MCP tool call reported `unsure` and increased the decision counter.

To uninstall:

```sh
codex plugin remove 8-ball@8-ball
```

For the fallback, run `codex mcp remove 8ball`. Then remove only the skill symlink that you created.

## Cursor

To install, use `.cursor-plugin/marketplace.json` in the Cursor plugin marketplace. Or copy the `mcpServers.8ball` entry from `plugin/.mcp.cursor.json` into `~/.cursor/mcp.json`. For a shared server, adjust its URL. The key uses `${env:EIGHTBALL_API_KEY}`.

To test:

```sh
cursor-agent --print --plugin-dir ./plugin --approve-mcps \
  "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

This is untested. Cursor Agent 2026.07.23-e383d2b accepts a per-run plugin, but the isolated test exited with authentication required before it connected. To try it, run `cursor-agent login`, then the command above. Make sure that a tool call occurs and that the decision counter of the server increases.

To uninstall, remove the 8-ball plugin in the Cursor marketplace. Or remove only the copied `mcpServers.8ball` entry and any skill link that you added.

## qwen-code

This is untested. To try it:

```sh
npm install -g @qwen-code/qwen-code@latest
qwen extensions install ./plugin
```

To test:

```sh
qwen -p "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

For a shared server, adjust `httpUrl` in `plugin/qwen-extension.json`. Its header reads `$EIGHTBALL_API_KEY`. A review compared the plugin with the [extension format](https://qwenlm.github.io/qwen-code-docs/en/users/extension/introduction/) and [HTTP transport fields](https://qwenlm.github.io/qwen-code-docs/en/users/features/mcp/). Runtime behavior is untested.

To uninstall:

```sh
qwen extensions uninstall 8-ball
```

## OpenCode v2

This is untested on v2. To try it, run `npm install -g @opencode/cli@latest` ([official v2 instructions](https://opencode.ai/v2/docs/)), then add the following to `~/.config/opencode/opencode.json`:

```json
{"mcp":{"servers":{"8ball":{"type":"remote","url":"http://127.0.0.1:8080/mcp","oauth":false,"headers":{"Authorization":"Bearer {env:EIGHTBALL_API_KEY}"}}}}}
```

```sh
ln -s "$PWD/plugin/skills/8-ball" ~/.agents/skills/8-ball
```

[Official MCP docs](https://opencode.ai/v2/docs/mcp-servers) show `mcp.servers` and `oauth: false` for bearer-only authentication. Start with the default Code Mode.

To test:

```sh
opencode mcp list
opencode run "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

Make sure that a decision tool call occurs and that the metric increases. If Code Mode cannot call the tool, record the failure first. Then try `"codemode": false` in that server entry.

To uninstall, remove only `mcp.servers.8ball` from the configuration and the skill symlink that you created.
