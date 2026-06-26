---
roder-api: minor
roder-ext-subagents: minor
---

# Subagent and swarm children inherit the parent workspace

Subagent (`task`) and agent-swarm children built their tool-execution context
with no handles, so any child file/shell/search tool failed with "workspace
handle is not available" and the child could not do real work. Children now
inherit the lead turn's workspace, remote workspace, process runner, and
context-artifact handles via the new `SubagentDispatcher::dispatch_with_context`
(the parent goal controller and trace sink are intentionally not inherited).
Each child still runs on its own child thread/turn id, so it operates on the
same repository as an independent agent rather than being confused with the
main-line thread.
