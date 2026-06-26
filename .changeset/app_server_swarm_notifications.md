---
roder-app-server: patch
---

# Forward agent-swarm events to remote app-server clients

Map the `AgentSwarmModeChanged`, `AgentSwarmStarted`, `AgentSwarmProgress`, and
`AgentSwarmCompleted` runtime events onto same-named JSON-RPC notifications
(`agentSwarm/modeChanged`, `agentSwarm/started`, `agentSwarm/progress`,
`agentSwarm/completed`) so SDK and remote app-server clients can observe a swarm
end-to-end, not just local TUI consumers (roadmap 104 follow-up). Per-child
detail continues to stream through the existing `turn/subagentTrace*` family.
