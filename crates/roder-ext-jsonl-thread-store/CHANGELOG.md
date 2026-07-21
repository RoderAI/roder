## 0.1.3 (2026-07-21)

### Fixes

#### Read lifecycle state without loading full threads

Thread stores can now load persisted extension state directly. Lifecycle-only
reads use that seam, so metadata-only thread reads do not need to project a
full event, turn, and item snapshot.

## 0.1.2 (2026-07-21)

### Fixes

#### Add bounded lifecycle recovery, cleanup proof, and shutdown diagnostics

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

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
