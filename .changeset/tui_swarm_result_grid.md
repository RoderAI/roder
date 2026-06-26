---
roder-tui: minor
---

# Render agent_swarm results as a compact child grid

The TUI timeline now renders an `agent_swarm` tool call with a compact swarm
summary line (`swarm: completed: 2, failed: 1`) grouped under the tool call, and
per-child outcome/item/agent-id rows when expanded, instead of the raw
`<agent_swarm_result>` XML block (roadmap 104, Task 5). Live per-child progress
continues to surface through the existing subagent trace surface.
