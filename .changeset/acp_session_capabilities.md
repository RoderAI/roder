---
roder-app-server: minor
roder-core: minor
roder-protocol: minor
roder: minor
roder-ext-mcp: minor
roder-ext-browser-use: patch
roder-tui: patch
---

# ACP session tools, controls, images and discovery

Support injected stdio and streamable HTTP MCP servers, image prompts, saved
session listing, model selection and durable session-specific permission modes.
Apply Paseo host instructions as thread developer instructions and restore
existing sessions with matching tool bindings. Close MCP connections on session
close and orderly CLI shutdown.

Native `thread/state` and `thread/set_mode` accept an optional `threadId` for
scoped mode access. Stdio MCP clients accept a working directory; HTTP MCP
clients expose explicit session shutdown.
