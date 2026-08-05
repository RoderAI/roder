---
roder-api: minor
roder-core: minor
roder-protocol: minor
roder-app-server: minor
roder-tui: minor
roder: patch
roder-ext-cursor: patch
roder-ext-subagents: minor
roder-extension-host: patch
roder-ext-task-subagent: patch
roder-ext-process-host: patch
roder-dynamic-workflows: patch
roder-sdk-typescript: minor
roder-sdk-python: minor
---

# Add Ultra mode as a first-class multi-agent mode for any model

Make Codex Ultra's proactive multi-agent policy a concrete Roder mode
(`/ultra`, `thread/set_ultra_mode`, `settings/get.ultraMode`,
`ultra/modeChanged`), available for every model — not only Sol/Terra Ultra
reasoning effort. Sol/Terra Ultra effort still maps to max wire effort and
enables proactive multi-agent without requiring the mode flag.

Also: `task` / `agent_swarm` children inherit the parent thread's live
provider+model (so SuperGrok stays on grok-4.5), and lane `max_concurrent`
can be raised per request for large fanouts instead of hard-failing at the
old scout cap of 4.
