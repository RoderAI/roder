---
roder-api: patch
roder-core: patch
roder-ext-xai: patch
roder: patch
---

# Recover from Grok prompt-length overflows and advertise xhigh

Detect xAI/Grok `maximum prompt length` (and related context-overflow) errors as
context-limit failures, then force-compact and retry the live turn in place
(with a second attempt that strips the last bulky item). Compaction summary
inference shrinks its head and falls back to deterministic summaries when the
summary request itself overflows. Grok reasoning catalogs now include `xhigh`.
