---
roder-ext-mysql-session: minor
---

Add tenant-scoped runtime ownership leases with database-clock expiry and monotonic generations. Expired and superseded owners cannot renew or release a replacement owner. Fence session, checkpoint, event, and artifact writes within owner-locked transactions; unbound handles cannot write once ownership is enabled for a tenant. This does not yet enable gateway failover or fence external actions.
