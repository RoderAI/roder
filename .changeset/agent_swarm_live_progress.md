---
roder-api: minor
roder-core: minor
roder-ext-subagents: minor
---

# Live agent_swarm progress events

Emit an `AgentSwarmProgress` `RoderEvent` each time a swarm child resolves,
carrying a running `completed/failed/aborted/total` snapshot (roadmap 104,
Task 1 follow-up). This lets a client render a live "N/total done" tick between
`AgentSwarmStarted` and `AgentSwarmCompleted` instead of only the final result.
The scheduler reports incremental progress through a new
`AgentSwarmProgressObserver`; the `agent_swarm` tool bridges it onto a runtime
`AgentSwarmProgressSink` supplied on the tool-execution context, which the
runtime backs with the event bus (and thread-event persistence). Children do
not publish progress; only the lead swarm does.
