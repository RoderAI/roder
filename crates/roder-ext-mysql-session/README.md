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
indexes of SHA-256 event-ID digests to both event tables. Long IDs remain supported. Startup serializes this DDL with a database-scoped
advisory lock; an interrupted migration resumes at the unfinished table. Existing
payloads and sequences are retained. The ALTER operations can rebuild/lock large
tables: schedule the upgrade with sufficient startup time and stop older writers
before admitting the new version. Old writers still supply runtime sequences and
must not run concurrently with the upgraded store. After upgrading, confirm the
migration version and test history restoration after restarting the runtime.

Do not roll back to writers that key records by runtime sequence. Roll back the
application with this store version retained, or restore a database backup with
writers stopped. Events overwritten before this upgrade cannot be recovered by
the migration.
