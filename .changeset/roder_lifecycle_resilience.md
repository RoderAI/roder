---
roder-api: minor
roder-app-server: minor
roder: minor
roder-config: minor
roder-core: minor
roder-dynamic-workflows: patch
roder-ext-claude-code: patch
roder-ext-jsonl-thread-store: patch
roder-ext-postgres-session: patch
roder-ext-process-host: patch
roder-ext-task-process: patch
roder-ext-task-subagent: patch
roder-ext-webwright: patch
roder-protocol: minor
roder-tasks: minor
roder-tui: patch
---

# Add bounded lifecycle recovery, cleanup proof, and shutdown diagnostics

Roder now persists redacted per-turn lifecycle records, reconciles interrupted
turns after restart, and reports bounded cleanup ownership rather than treating
an aborted runtime task as proof that provider work was reaped. Local process
tasks drain through graceful signal, forced kill, and reap; remote tasks use the
remote runner cancellation API; and the Claude Code provider uses a vendored SDK
cleanup path with offline real-child regression coverage.

The app-server adds lifecycle notifications, `runtime/drain`, and
`lifecycle/metrics`; the CLI and TUI expose durable recovery state. A shared
`[lifecycle]` configuration controls shutdown budgets, task policy, bounded
process diagnostics, and compatible legacy shutdown fallbacks.
