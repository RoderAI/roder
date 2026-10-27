# Owned agent worktrees

`spawn_agent` accepts `workspace_isolation: "shared" | "worktree"`. The default
is `shared`. This controls the filesystem independently of `fork_turns`, which
only chooses conversation history.

For example, an agent can delegate with:

```json
{
  "task_name": "implement_parser",
  "message": "Implement the parser and report the resulting branch.",
  "fork_turns": "all",
  "workspace_isolation": "worktree"
}
```

Worktree isolation requires the Git fork provider, a durable thread store, and a
local, clean source checkout. It creates a new branch from committed HEAD;
uncommitted changes are never copied. Dirty sources fail closed (Roder-owned
`.roder/` runtime state is exempt). Remote-runner parents are rejected rather
than silently executing on the host.

The child retains the parent's policy and tool filters. Its thread metadata,
inherited turn workspace, and default coding-tool workspace all point to the
new checkout. This is filesystem workflow isolation, **not a security sandbox**:
existing path permissions and approvals remain authoritative.

The spawn result includes `workspace` and `workspace_fork`. Thread metadata also
persists `workspace_fork`, including an `ownership` object with
`creatorThreadId` and `ownerThreadId`. Git stores the durable ownership record
in its worktree administration directory, outside the editable checkout.
Concurrent/restarted creation requests for the same owner reuse the existing
owned fork rather than creating another one.

## Discovery and cleanup

Existing `forks/list` includes external registered worktrees, but those entries
omit `ownership`. Roder never adopts or removes such a worktree, even when its
branch or path looks like a Roder fork. A missing or invalid ownership record
fails closed. Retrying creation cannot transfer ownership to a different creator.

Completed/interrupted agents keep their worktrees for review and follow-up.
There is no automatic merge or successful-task cleanup. Use the existing
path-confirmed `thread/remove_fork` or `forks/remove` methods for explicit
cleanup. Git removal is never forced: dirty or locked worktrees are retained.
Branches are retained after removal, preserving committed changes.
Runtime removal also rejects an agent checkout while its owner has an active
turn; interrupt and await completion before cleanup.

If initial agent setup fails after allocating a checkout, Roder attempts
non-force cleanup of that owned checkout. If it cannot be removed safely,
the checkout remains available for inspection; cleanup never deletes another
thread's or an external worktree.

Regression coverage: `roder-ext-git/tests/fork_provider.rs` and
`roder-core/src/runtime/agent_workspace_tests.rs` exercise ownership persistence,
external checkout rejection, duplicate creation, dirty refusal, clean startup
rollback, and two child agents writing through runtime tool routing.
