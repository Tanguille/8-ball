# MCP clients

Start the server as shown in the [quickstart](../README.md#quickstart). Run the commands here from the repository root. Claude Code uses `EIGHTBALL_URL` to find the server.

## Claude Code

Install the plugin.

```sh
claude plugin marketplace add .
claude plugin install 8-ball@8-ball
```

Set `EIGHTBALL_API_KEY` if your server uses an API key. A local server with no key accepts the empty bearer header.

Ask a question and read the decision counter.

```sh
claude -p --plugin-dir ./plugin \
  --allowedTools 'mcp__plugin_8-ball_8ball__decision_ask,mcp__plugin_8-ball_8ball__decision_triage' \
  "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
curl -s http://127.0.0.1:8080/metrics
```

Claude Code 2.1.317 returned a verdict and raised the counter on 2026-11-03. In strict standalone mode (`--mcp-config plugin/.mcp.json`), tool names start with `mcp__8ball__`.

To remove the plugin, run this command.

```sh
claude plugin uninstall 8-ball@8-ball
```

## Codex

For a shared server, edit the URL in `plugin/.mcp.codex.json` before you install the plugin. Codex does not expand environment variables in URLs. Set the key in `EIGHTBALL_API_KEY`.

Install the plugin.

```sh
codex plugin marketplace add .
codex plugin add 8-ball@8-ball
```

To add MCP directly, run these commands.

```sh
codex mcp add 8ball --url http://127.0.0.1:8080/mcp --bearer-token-env-var EIGHTBALL_API_KEY
ln -s "$PWD/plugin/skills/8-ball" ~/.agents/skills/8-ball
```

Test with a temporary MCP configuration.

```sh
codex exec --ephemeral \
  -c 'mcp_servers.8ball.url="http://127.0.0.1:8080/mcp"' \
  -c 'mcp_servers.8ball.tools.decision_ask.approval_mode="approve"' \
  'Call decision_ask with {"state":"disk 98% full on /var","questions":[{"id":"urgent","type":"noul","instructions":"Is this alert urgent?"}]}, then report the verdict.'
```

The test override approves one tool. In an installed session, approve calls as they come up. See the [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference).

The temporary test returned `unsure` and raised the counter on Codex CLI 0.166.0 on 2026-11-03. The marketplace installation still needs a test.

To remove the plugin, run this command.

```sh
codex plugin remove 8-ball@8-ball
```

If you added MCP directly, run `codex mcp remove 8ball`. Then remove only the skill symlink that you created.

## Cursor

To install the plugin, use `.cursor-plugin/marketplace.json` in the Cursor plugin marketplace. To add MCP directly, copy the `mcpServers.8ball` entry from `plugin/.mcp.cursor.json` into `~/.cursor/mcp.json`.

For a shared server, change the URL. The key comes from `${env:EIGHTBALL_API_KEY}`.

Test the plugin.

```sh
cursor-agent --print --plugin-dir ./plugin --approve-mcps \
  "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

Cursor Agent 2026.07.23-e383d2b stopped for authentication before it connected. To finish the test, run `cursor-agent login` and retry. Look for a tool call and a higher decision count.

To uninstall, remove the 8-ball plugin in the Cursor marketplace. If you added MCP directly, remove only the copied `mcpServers.8ball` entry and any skill link that you added.

## qwen-code

The connection test is still pending. Install the extension to try it.

```sh
npm install -g @qwen-code/qwen-code@latest
qwen extensions install ./plugin
```

For a shared server, adjust `httpUrl` in `plugin/qwen-extension.json`. Its header reads `$EIGHTBALL_API_KEY`.

Test the extension.

```sh
qwen -p "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

We compared the extension configuration with the [extension format](https://qwenlm.github.io/qwen-code-docs/en/users/extension/introduction/) and [HTTP transport fields](https://qwenlm.github.io/qwen-code-docs/en/users/features/mcp/).

To remove the extension, run this command.

```sh
qwen extensions uninstall 8-ball
```

## OpenCode v2

OpenCode v2 still needs a connection test. Install it with `npm install -g @opencode/cli@latest`, then add this entry to `~/.config/opencode/opencode.json`. See the [official v2 instructions](https://opencode.ai/v2/docs/).

```json
{"mcp":{"servers":{"8ball":{"type":"remote","url":"http://127.0.0.1:8080/mcp","oauth":false,"headers":{"Authorization":"Bearer {env:EIGHTBALL_API_KEY}"}}}}}
```

```sh
ln -s "$PWD/plugin/skills/8-ball" ~/.agents/skills/8-ball
```

The [OpenCode MCP docs](https://opencode.ai/v2/docs/mcp-servers) describe `mcp.servers` and `oauth: false` for authentication with a bearer key. Start with the default Code Mode.

List the MCP servers and ask a test question.

```sh
opencode mcp list
opencode run "Use 8-ball to decide whether 'disk 98% full on /var' is urgent."
```

Look for the tool call and a higher decision count. If Code Mode cannot call the tool, record the failure. Then try `"codemode": false` in the server entry.

To uninstall, remove only `mcp.servers.8ball` from the configuration and the skill symlink that you created.
