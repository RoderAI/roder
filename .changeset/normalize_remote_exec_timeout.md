---
roder-tools: patch
---

# Keep remote exec timeout semantics honest

Normalize remote one-shot commands to a finite ten-minute maximum before the
provider request is built, so the local outer timeout, runner lease, and tool
result metadata all report and enforce the same bound during multi-hour turns.
