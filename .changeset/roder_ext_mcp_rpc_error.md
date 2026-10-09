---
roder-ext-mcp: minor
---

# MCP client exposes the server's JSON-RPC error as a typed error

Adds the typed `McpRpcError` (code and message), returned in the anyhow error chain when a server answers a request with a JSON-RPC error. Callers can tell a server that said no from one that is gone or silent. Error text is unchanged.
