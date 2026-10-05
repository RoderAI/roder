## 0.4.0 (2026-10-05)

### Breaking Changes

#### Move MySQL schema setup out of runtime connections. Run `roder-mysql-migrate`

with `RODER_MYSQL_SESSION_URL` as a release step before starting workers.
Session-store connections now check the schema version with a primary-key read
and never acquire migration locks or issue DDL.

## 0.3.0 (2026-10-03)

### Breaking Changes

#### Add tenant-scoped runtime ownership leases with database-clock expiry and monotonic generations. Expired and superseded owners cannot renew or release a replacement owner. Fence session, checkpoint, event, and artifact writes within owner-locked transactions; unbound handles cannot write once ownership is enabled for a tenant. Add an optional monotonic runtime execution lease: reject turn/tool admission after loss, recheck after approval waits, and close hosted sockets instead of delivering stale-owner notifications. Add bounded host-backed renewal supervision that revokes authority and drains local work on loss or uncertainty. Already dispatched external actions still require host-level reconciliation after a crash.

Add one-hop authenticated owner forwarding and discard revoked cached runtimes so reconnects can resolve ownership again. Hosts must supply a trusted owner endpoint and shared replica authentication/policy.

Add idle owner sealing that waits for admitted turns, tool futures, and cleanup without cancellation, followed by bounded durable release. Hosts must stop new inbound work and only treat confirmed release as a handoff receipt.

Add replica pool drain admission and release polling. Preserve active-work recovery messages, reject new work, keep readiness separate from liveness, and retain failed release receipts. Count forwarded sockets so an empty local runtime pool cannot falsely report a fully drained replica.

Expose an optional, bounded same-port HTTP lifecycle handler for host-authenticated rollout commands. Update the hosted distribution to leave it disabled by default.

Drain forwarded sockets through an authenticated owner registration without consuming repeated user request-rate tokens. Preserve active browser results, release only idle owners, cancel registrations on rollback, and reconnect read-only subscriptions without retiring their owner.

Update dependent distributions and the TUI for the breaking hosted options, session-store configuration, and MySQL configuration module APIs.

### Fixes

- Preserve event history across runtime restarts and concurrent writers by allocating durable MySQL sequence numbers and deduplicating by event identity. Upgrade existing event tables without replacing stored payloads.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.2 (2026-07-21)

### Fixes

#### Read lifecycle state without loading full threads

Thread stores can now load persisted extension state directly. Lifecycle-only
reads use that seam, so metadata-only thread reads do not need to project a
full event, turn, and item snapshot.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
