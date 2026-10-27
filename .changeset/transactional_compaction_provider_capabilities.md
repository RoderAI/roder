---
roder-api: minor
roder-config: minor
roder-core: patch
roder: patch
roder-extension-host: minor
roder-ext-openai-responses: minor
---

# Commit compaction only after completion and configure custom Responses capabilities

Failed or interrupted native compaction no longer installs an incomplete history
boundary. Retries preserve original context while reported usage remains billable.
The same transaction boundary applies to automatic compaction in ordinary turns,
retaining subsequent messages and eager tool-call results exactly once on replay
and restart.

Custom Responses providers can opt into V2 remote compaction and cap hosted search
at cached results with typed provider capabilities, without changing omitted defaults.
