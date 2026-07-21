---
roder-api: minor
roder-core: patch
roder-ext-jsonl-thread-store: patch
roder-ext-postgres-session: patch
roder-ext-mysql-session: patch
---

# Read lifecycle state without loading full threads

Thread stores can now load persisted extension state directly. Lifecycle-only
reads use that seam, so metadata-only thread reads do not need to project a
full event, turn, and item snapshot.
