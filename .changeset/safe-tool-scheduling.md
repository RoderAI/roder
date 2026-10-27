---
roder-api: minor
roder-core: minor
roder-tools: minor
roder-ext-mcp: minor
roder-ext-subagents: minor
---

# Schedule shared reads safely around exclusive tools

Use one conservative execution classification for streamed reads and ordinary
tool batches. Keep mutations exclusive and ordered, retain remote workspace
fences, and require explicitly trusted MCP tools before accepting read-only
scheduling hints. Scheduling never bypasses policy or workspace authorization.
