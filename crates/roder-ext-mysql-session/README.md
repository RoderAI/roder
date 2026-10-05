# roder-ext-mysql-session

`roder-ext-mysql-session` is the MySQL session store for [Roder](https://roder.sh).

## What It Does

It persists tenant-scoped Roder threads and context artifacts in MySQL.

## How It Fits Into Roder

Roder is an agentic software development system with a Rust CLI/TUI, a JSON-RPC app-server, SDKs, package resources, and first-party runtime extensions. This package is released as part of that workspace so downstream users can depend on the same component boundaries that Roder itself uses.

## Links

- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```

## Durable event history

MySQL allocates persisted event ordering independently of runtime counters. Both
runtime and item events deduplicate by their event IDs; replay does not replace an
existing payload. Loaded event sequences reflect stable database allocation order, not a cross-process causal clock. Callers must await causally dependent appends.

Schema version 2 adds database sequence allocation and unique event identity
indexes of SHA-256 event-ID digests to both event tables. Long IDs remain supported.
Run schema setup once as a release step, before admitting workers:

```sh
# Supply the database URL through your secret environment, not command arguments.
cargo run -p roder-ext-mysql-session --bin roder-mysql-migrate
```

The command reads `RODER_MYSQL_SESSION_URL`. You can also install the command with
`cargo install roder-ext-mysql-session --bin roder-mysql-migrate`.
Embedders can call `schema::migrate(&pool)` from
their release tooling. Runtime connections only read the required migration
version and fail with setup instructions if it is missing. They never perform
DDL or acquire a schema advisory lock; runtime users only need data privileges.
Provision isolated test databases with the same command before running the live
store, ownership, fencing, or gateway suites.

The explicit migration serializes event-table DDL with a database-scoped
advisory lock; an interrupted migration resumes at the unfinished table. Existing
payloads and sequences are retained. The ALTER operations can rebuild/lock large
tables: schedule the upgrade as a maintenance operation and stop older writers
before admitting the new version. Old writers still supply runtime sequences and
must not run concurrently with the upgraded store. After upgrading, confirm the
migration version and test history restoration after restarting the runtime.

Do not roll back to writers that key records by runtime sequence. Roll back the
application with this store version retained, or restore a database backup with
writers stopped. Events overwritten before this upgrade cannot be recovered by
the migration.


## Runtime ownership primitive

`claim_runtime_owner`, `renew_runtime_owner`, `release_runtime_owner`, and
`runtime_owner` use a tenant-scoped MySQL row with a process UUID, internal socket
address, and monotonically increasing generation. The database clock determines
expiry. A competing claim observes the existing live owner; an expired owner
cannot renew. Release retains the generation, so an old handle cannot release a
later owner. Tenant keys compare byte-for-byte. TTLs are bounded to 1–300 seconds.
Renewal never shortens an existing lease.

Use `with_runtime_owner(&lease)` to derive the runtime's store handle, then install
it with `MysqlSessionExtension::from_store` to retain the fence in the runtime's
thread-store factory. Every
session, event, checkpoint, and artifact write validates the live generation and
holds the ownership row lock through commit. New ownership cannot be granted
between validation and the write. Once a tenant has claimed ownership, unbound
handles cannot write, even after the lease is released. Read access remains
available for recovery. All writer binaries must support this protocol before
enabling ownership; older binaries do not enforce these transaction guards.

This is not enabled gateway HA. A routing observation is not execution authority.
Integration must fence tool actions,
stop admission on lease loss, reconcile uncertain external actions before replay,
and quiesce work before release. A lost database response is an unknown outcome:
reconcile the process UUID and generation; do not start another runtime blindly.
The store's dedicated executor can finish an operation after its caller cancels.
Use a conservative local monotonic deadline measured from before the lease request
when deciding whether work can continue; the returned database timestamp is not a
local-clock deadline. Gateway routing and these execution guards remain separate
integration work.

Real database checks:

```sh
RODER_MYSQL_TEST_URL=mysql://... cargo test -p roder-ext-mysql-session --test runtime_ownership -- --ignored
```


The host can bind `roder_core::RuntimeExecutionLease` with
`Runtime::with_execution_lease` before publishing the runtime. Its deadline uses
local monotonic time and must be extended only after a confirmed renewal of the
same durable generation. Expiry and explicit revocation are irreversible for
that runtime. Turn admission, tool entry, and dispatch after approval waits
check this lease. Hosted WebSockets close when it is lost and drop queued
notifications from the stale owner. Operations already in flight still require
outcome reconciliation; these checks do not implement remote action fencing,
owner routing. `AppServer::supervise_runtime_lease` supplies bounded renewal
supervision through a host-provided `HostedRuntimeLeaseBackend`. The backend
must confirm the exact durable generation; uncertainty, timeout, or rejection
revokes the local guard and starts bounded runtime cleanup. Dropping the server
also revokes surviving runtime clones. Supervision does not release the durable
lease or implement graceful owner transfer; those remain host coordination work.
