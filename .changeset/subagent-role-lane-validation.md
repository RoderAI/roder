---
roder-api: patch
roder-core: patch
roder-ext-subagents: patch
---

# Clarify subagent role selection and native full-history labels

Model-facing subagent tools now advertise configured roles, reject lane names
used as roles before fanout, and report lane/tool incompatibilities before a
child agent runs. Native `spawn_agent` full-history forks now accept their
advisory `agent_type` label while continuing to reject model, provider, and
reasoning overrides.
