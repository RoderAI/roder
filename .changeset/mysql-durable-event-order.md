---
roder-ext-mysql-session: patch
---

Preserve event history across runtime restarts and concurrent writers by allocating durable MySQL sequence numbers and deduplicating by event identity. Upgrade existing event tables without replacing stored payloads.
