---
roder-tui: minor
---

# Live agent_swarm progress in the timeline

Render a live "swarm: N/total done" tick on the `agent_swarm` tool row while the
swarm is running, fed by `AgentSwarmProgress` events, instead of an empty row
until the final result. Once the `<agent_swarm_result>` arrives, the row swaps to
the existing compact child grid. When any children fail or abort, the live line
also shows the `(ok, failed, aborted)` breakdown.
