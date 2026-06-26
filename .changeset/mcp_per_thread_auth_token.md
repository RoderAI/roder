---
roder-api: minor
roder-protocol: minor
roder-app-server: minor
roder-ext-mcp: minor
---

# Per-thread MCP bearer token

Let a remote client scope a thread's MCP tool calls to a specific identity (for
Vex: a per-user, per-organization capability token). The client forwards the
token via a new `mcpAuthToken` field on `thread/start`; the app-server records
it in an in-memory `roder_api::mcp_auth` registry keyed by thread id, and the
MCP tool extension reads it during execution to authenticate that thread's tool
calls (falling back to the process default when absent). Tokens are short-lived
and re-supplied on each `thread/start`.
