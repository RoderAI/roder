---
roder: patch
roder-core: patch
roder-extension-host: patch
roder-ext-openai-responses: patch
---

# Preserve native compaction across OAuth and routed turns

Codex OAuth sessions require provider-owned compaction. Defer automatic
compaction until routing selects the provider, and keep local fallback available
for custom Responses providers that do not implement OpenAI native compaction.
