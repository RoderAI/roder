---
roder-core: patch
roder: patch
---

When resuming after Stop or a runtime restart, represent unanswered previous
tool calls as unknown errors in inference context. Preserve the original
transcript and completed results, and tell the model not to repeat those calls.
