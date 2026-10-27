---
roder-api: minor
roder-core: patch
roder-protocol: minor
roder-app-server: minor
roder-ext-jsonl-thread-store: minor
roder-sdk-typescript: patch
roder-sdk-python: patch
roder: patch
---

# Read thread history in bounded pages without loading complete snapshots

Add the read-only `thread/history/read` API with chronological item pages, opaque
exclusive cursors, item and byte budgets, and complete turn metadata. The local
JSONL store maintains a derived seek/projection index, allowing warm page reads
to deserialize only selected items instead of hydrating entire threads.

Index updates preserve streamed deltas and terminal outcomes. Older logs rebuild
the disposable index one committed record at a time. Unsupported stores report
their missing paging capability explicitly rather than silently loading full
snapshots. Generated method manifests expose the new API to SDK consumers.
