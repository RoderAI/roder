---
roder-api: minor
roder-core: minor
---

# Agent-swarm lifecycle events on the event bus

Emit `AgentSwarmStarted` (with the child count) and `AgentSwarmCompleted` (with
completed/failed/aborted counts) `RoderEvent`s when the `agent_swarm` tool runs,
so any app-server/SDK/TUI client can observe a swarm as a whole rather than only
the per-child `Subagent*` traces (roadmap 104, Task 1). The runtime emits these
around tool routing; existing notification mappers fall through their catch-all
arms, so no client breaks.
