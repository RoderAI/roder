---
roder-ext-mysql-session: minor
roder-core: minor
roder-app-server: minor
---

Add tenant-scoped runtime ownership leases with database-clock expiry and monotonic generations. Expired and superseded owners cannot renew or release a replacement owner. Fence session, checkpoint, event, and artifact writes within owner-locked transactions; unbound handles cannot write once ownership is enabled for a tenant. Add an optional monotonic runtime execution lease: reject turn/tool admission after loss, recheck after approval waits, and close hosted sockets instead of delivering stale-owner notifications. Add bounded host-backed renewal supervision that revokes authority and drains local work on loss or uncertainty. Gateway routing, graceful owner transfer, and fencing of in-flight external actions remain separate integration work.

Add one-hop authenticated owner forwarding and discard revoked cached runtimes so reconnects can resolve ownership again. Hosts must supply a trusted owner endpoint and shared replica authentication/policy.
