# Worktree Conversation Forks

Roder can fork an active conversation into a **child thread backed by a local
Git worktree**, so an experiment can proceed in an isolated writable workspace
while the parent thread and workspace stay untouched.

Terminology (these are Roder conversation/workspace forks, not GitHub
repository forks):

- **Conversation fork** — a new thread derived from an existing thread's
  transcript and metadata.
- **Workspace fork** — the writable filesystem root attached to that thread.
- **Native worktree fork** — this local MVP: the workspace fork is a
  `git worktree` of the parent repository.

## What happens on fork

1. The parent workspace must be a Git repository with **no uncommitted
   changes** (tracked or untracked). Dirty parents fail closed so the child
   never silently diverges from what you see. Roder-owned state under
   `.roder/` is exempt.
2. Roder creates a worktree under `<repo-root>/.roder/worktrees/<name>` (then
   `<name>-2`, `<name>-3`, … on collision) on a new branch
   `roder/fork/<name>`, recording the source branch and commit (a detached
   HEAD records the commit only).
3. A child thread is created whose workspace is the worktree. It inherits the
   parent's provider/model/tool settings and is seeded with the parent
   conversation history (turn lifecycle and transcript records only — tool
   calls are **never replayed**). `--from-turn` forks at a specific turn.
4. All subsequent tool reads/writes, shell commands, and search in the child
   resolve against the child worktree.

Provenance persists in thread metadata (`parentThreadId`,
`forkedFromTurnId`, `worktreeFork` with fork id, backend, paths, branch,
source commit, status, and cleanup policy) and survives restarts in both the
JSONL and PostgreSQL thread stores.

## Surfaces

TUI:

```text
/fork <name>                  fork this conversation and switch to the child
/fork status                  show fork provenance for the current thread
/fork remove <worktree-path>  explicitly remove the fork worktree
```

CLI:

```sh
roder thread fork-worktree <thread-id> --name <name> [--from-turn <turn-id>]
roder thread fork-status <thread-id>
roder thread remove-worktree-fork <thread-id> --confirm-path <worktree-path>
```

App-server JSON-RPC:

- `thread/fork_worktree` `{ threadId, name, fromTurnId? }` → child `Thread`
  (its `cwd` is the worktree), `fork` provenance, and `warnings`.
- `thread/fork_status` `{ threadId }` → `parentThreadId`, `forkedFromTurnId`,
  `fork`, and `worktreeMissing`.
- `thread/remove_worktree_fork` `{ threadId, confirmPath }` → updated `fork`
  with `status: "removed"`.

`thread/list` and `thread/read` include `parentThreadId` and `worktreeFork`
on forked threads so clients can label parent/child relationships.

## Safety and cleanup

- Removal is **explicit and path-confirmed**: the request must repeat the
  exact worktree path. Only paths that Git reports as registered worktrees of
  the source repository are ever removed (`git worktree remove` +
  `git worktree prune`); arbitrary directories and the primary worktree are
  refused. The fork branch is kept for provenance, and the child conversation
  remains readable with `status: "removed"`.
- If a worktree disappears out-of-band (manual deletion), the child thread
  **fails closed before any write**: turn startup reports the fork id and
  missing path, and `thread/fork_status` returns `worktreeMissing: true`.
  Restore the directory (`git worktree add`) or remove the fork.
- Forking a non-Git workspace fails with a clear error; remote/sandbox-backed
  forks are the future provider-neutral expansion (roadmap phase 81), which
  will reuse this conversation-fork user model while swapping the workspace
  materialization backend.

## Events

Fork lifecycle emits runtime audit events: `thread.fork_requested`,
`thread.forked`, `thread.fork_failed`, and `thread.fork_removed`, plus the
normal `thread.created` for the child.

## Local verification

```sh
cargo test -p roder-ext-git --test worktree
cargo test -p roder-core --test conversation_fork
cargo test -p roder-app-server --features e2e-tests --test fork_worktree
cargo test -p roder-tui commands
```

All tests create temporary Git repositories; no network or provider
credentials are required.
