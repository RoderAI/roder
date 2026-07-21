---
roder-ext-mcp: minor
---

# Fail closed when scoped MCP authentication is required

MCP servers can now require a thread-scoped bearer token for tool execution
while continuing to use their configured process credential for startup tool
discovery. Calls without a thread credential are rejected locally before any
HTTP request, preventing shared hosted services from falling back to a
process-wide identity.
