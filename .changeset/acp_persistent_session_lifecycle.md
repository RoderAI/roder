---
roder: patch
roder-app-server: minor
roder-core: minor
roder-protocol: minor
---

# Restore and close persistent ACP sessions

Support ACP load with history replay, resume without replay, and close with turn
drain and live resource release. Preserve persisted conversations and validate
their workspace when reconnecting. Add native thread/unload and cancel pending
permission callbacks during close.
