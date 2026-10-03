---
roder-ext-mysql-session: minor
---

Add tenant-scoped runtime ownership leases with database-clock expiry and monotonic generations. Expired and superseded owners cannot renew or release a replacement owner. This storage primitive does not yet enable gateway failover or fence external actions.
