# Workspace Forks

Roder can fork a workspace into an isolated writable branch of execution
through a provider-neutral `ForkProvider` API (roadmap phase 81, which
absorbed the phase-90 Git-worktree MVP). A **fork** is a Roder-owned
concept — a writable workspace copy or session derived from a source
workspace, backing a thread, subagent lane, task branch, or experiment. It
is not an inference provider and not a GitHub repository fork.

## Providers

| Provider id | Backing | Notes |
| --- | --- | --- |
| `git-worktree` (default) | `git worktree` + branch `roder/fork/<name>` | Worktrees under `<repo>/.roder/worktrees/<name>`; dirty sources fail closed (Roder-owned `.roder/` state exempt); non-git workspaces are rejected with a clear error. |
| `rift` | [Rift](https://github.com/anomalyco/rift) copy-on-write snapshots | Shells out to a configured `rift` binary (`RODER_RIFT_BIN`); upstream is pre-1.0, so offline tests run against a fake binary and live checks are gated behind `RODER_RIFT_LIVE=1` + `RIFT_BIN`. |
| `remote-runner` | A fresh remote-runner session (`RemoteRunnerForkAdapter`) | `remote_compute = true`; file/process operations stay on the `RemoteRunnerSession` contract; removal closes the session. Not registered by default — hosts construct it with a destination. |

Provider trust boundaries: fork ids are provider-scoped (the absolute
workspace path for local providers, the session id for remote ones),
creation fails closed on dirty sources by default, and removal is always
explicit and path-confirmed — the request must repeat the exact fork
workspace. Fork metadata never carries secrets.

## Conversation forks (threads)

Forking a thread creates a child thread attached to a fresh workspace fork
of the parent workspace, seeded with the parent conversation history (turn
lifecycle and transcript records only — tool calls are never replayed).
Provenance persists in thread metadata (`parentThreadId`,
`forkedFromTurnId`, `workspaceFork`) across JSONL and PostgreSQL stores,
and a fork whose workspace disappears out-of-band fails closed before any
write.

TUI:

```text
/fork <name>                   fork this conversation and switch to the child
/fork status                   show fork provenance for the current thread
/fork remove <workspace-path>  explicitly remove the fork workspace
```

CLI:

```sh
roder thread fork <thread-id> --name <name> [--from-turn <id>] [--provider <id>]
roder thread fork-status <thread-id>
roder thread remove-fork <thread-id> --confirm-path <workspace-path>

roder forks providers
roder forks list [--source <path>] [--provider <id>]
roder forks create --name <name> [--source <path>] [--provider <id>]
roder forks remove <fork-id> --confirm-workspace <path> [--provider <id>]
```

## App-server JSON-RPC

- `thread/fork` `{ threadId, name, fromTurnId?, provider?, providerConfig? }`
  → child `Thread` (its `cwd` is the fork workspace), `fork`
  (`WorkspaceFork`), `warnings`.
- `thread/fork_status` `{ threadId }` → `parentThreadId`,
  `forkedFromTurnId`, `fork`, `workspaceMissing`.
- `thread/remove_fork` `{ threadId, confirmPath }` → fork with
  `status: "removed"` (the conversation stays readable).
- `forks/providers/list` → registered provider descriptors/capabilities.
- `forks/list` `{ sourceWorkspace, provider? }` → forks of a workspace.
- `forks/create` `{ sourceWorkspace, name?, provider?, providerConfig? }`.
- `forks/remove` `{ forkId, provider?, confirmWorkspace }`.

`thread/list`/`thread/read` include `parentThreadId` and `workspaceFork`
on forked threads.

## Configuration

```toml
[forks]
default_provider = "git-worktree"  # or "rift"
# base_dir = "/path/for/fork-workspaces"
```

Env overrides: `RODER_FORK_PROVIDER`, `RODER_FORK_BASE_DIR`,
`RODER_RIFT_BIN`. Explicit request parameters always win over config.

## Events

Fork lifecycle emits `thread.fork_requested`, `thread.forked`,
`thread.fork_failed`, and `thread.fork_removed`, plus the normal
`thread.created` for child threads.

## Local verification

```sh
cargo test -p roder-api --test fork_api
cargo test -p roder-ext-git --test fork_provider
cargo test -p roder-ext-fork-rift
cargo test -p roder-core --test conversation_fork
cargo test -p roder-core --test fork_isolation
cargo test -p roder-app-server --features e2e-tests --test forks
```

Everything runs offline with temp Git repositories and fake binaries; the
isolation tests prove two agents editing the same source repo through
separate forks have disjoint write sets with the source untouched.
