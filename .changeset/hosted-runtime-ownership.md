---
roder-ext-mysql-session: minor
roder-core: minor
roder-app-server: minor
---

Add tenant-scoped runtime ownership leases with database-clock expiry and monotonic generations. Expired and superseded owners cannot renew or release a replacement owner. Fence session, checkpoint, event, and artifact writes within owner-locked transactions; unbound handles cannot write once ownership is enabled for a tenant. Add an optional monotonic runtime execution lease: reject turn/tool admission after loss, recheck after approval waits, and close hosted sockets instead of delivering stale-owner notifications. Gateway routing, renewal supervision, and fencing of in-flight external actions remain separate integration work.
