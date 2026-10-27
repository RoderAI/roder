---
roder-api: minor
roder-core: minor
roder-ext-git: minor
roder-app-server: patch
roder-ext-fork-rift: patch
roder-ext-jsonl-thread-store: patch
---

# Run collaboration agents in durable, owned Git worktrees

Add opt-in `workspace_isolation: "worktree"` to `spawn_agent`, persisting creator
and child ownership and routing child turns and coding tools to their isolated
checkout. Reuse owned worktrees on creation retries; refuse adoption or deletion
of external registered worktrees. Keep successful worktrees for review and use
non-force, path-confirmed cleanup with conservative rollback on failed starts.
