---
roder-ext-browser-use: minor
roder-ext-mcp: minor
roder-extension-host: minor
roder-config: minor
roder: minor
---

# Add browser-use as an opt-in browser provider over stdio MCP

`roder --browser-use` or `[browser_use] enabled = true` exposes the open-source
browser-use local MCP server as `browser_use_*` tools. Roder launches it with
`uvx --from 'browser-use[cli]==0.13.10' browser-use --mcp` on the first call,
with a visible browser by default (`headless = true` to hide it), an
allowlisted environment, and only the OpenAI/Anthropic keys Roder already
holds, which are scrubbed from every result and error. Clicks and typing ask
for approval in default mode, the autonomous agent tool asks in default and
accept-all mode, and plan mode denies both. Page content is labeled untrusted.

`roder-ext-mcp` gains `McpStdioClient`, a stdio transport that runs the
server in its own process group, answers server pings, keeps a redacted
stderr tail for errors, and stops the whole group (including a browser the
server started) on shutdown, on drop, and, through a pid guard, when Roder
dies.
