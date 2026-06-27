---
roder-ext-claude-code: minor
roder-extension-host: patch
---

# Let the Claude Code provider use the "Claude in Chrome" browser tools

The `claude-code` provider can now register against the local Claude Code
"Claude in Chrome" (CFC) integration so the model can drive the user's real
browser through the CLI's `mcp__claude-in-chrome__*` tools.

When enabled, the provider spawns `claude` with `CLAUDE_CODE_ENABLE_CFC=1` (so
the CLI wires its browser MCP server even in the SDK's headless/streaming mode)
and blanks `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN` for that child so the CLI
uses claude.ai subscription auth — a prerequisite for the Chrome integration to
connect. It pre-authorizes the browser tools through the SDK `can_use_tool`
callback (all other unmapped CLI tools stay denied) and surfaces the
CLI-executed browser tool calls as hosted tool calls
(`HostedToolCallStarted`/`HostedToolCallCompleted`) so they show in the UI
without the runtime trying to re-run a tool it never registered.

Enablement resolves from `ClaudeCodeConfig::enable_claude_in_chrome`, then the
`RODER_CLAUDE_CODE_ENABLE_CHROME`/`CLAUDE_CODE_ENABLE_CHROME` env vars, then
auto-detection of a paired/enabled Chrome extension in the local Claude Code
config. No `claude-agent-sdk` change was required.
