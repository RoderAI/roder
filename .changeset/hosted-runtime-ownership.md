---
roder-ext-mysql-session: minor
roder-core: minor
roder-app-server: minor
roder-extension-host: minor
roder-dist-hosted: patch
---

Add tenant-scoped runtime ownership leases with database-clock expiry and monotonic generations. Expired and superseded owners cannot renew or release a replacement owner. Fence session, checkpoint, event, and artifact writes within owner-locked transactions; unbound handles cannot write once ownership is enabled for a tenant. Add an optional monotonic runtime execution lease: reject turn/tool admission after loss, recheck after approval waits, and close hosted sockets instead of delivering stale-owner notifications. Add bounded host-backed renewal supervision that revokes authority and drains local work on loss or uncertainty. Graceful owner transfer, and fencing of in-flight external actions remain separate integration work.

Add one-hop authenticated owner forwarding and discard revoked cached runtimes so reconnects can resolve ownership again. Hosts must supply a trusted owner endpoint and shared replica authentication/policy.

Add idle owner sealing that waits for admitted turns, tool futures, and cleanup without cancellation, followed by bounded durable release. Hosts must stop new inbound work and only treat confirmed release as a handoff receipt.

Add replica pool drain admission and release polling. Preserve active-work recovery messages, reject new work, keep readiness separate from liveness, and retain failed release receipts. Count forwarded sockets so an empty local runtime pool cannot falsely report a fully drained replica.

Expose an optional, bounded same-port HTTP lifecycle handler for host-authenticated rollout commands. Update the hosted distribution to leave it disabled by default.

Drain forwarded sockets through an authenticated owner registration without consuming repeated user request-rate tokens. Preserve active browser results, release only idle owners, cancel registrations on rollback, and reconnect read-only subscriptions without retiring their owner.
