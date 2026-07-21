## 0.1.3 (2026-07-21)

### Fixes

#### Bound and cancel detached remote commands

Remote command requests can now carry a wall-clock process lease. Remote shell
and exec tools request provider cancellation when they time out or are dropped
by turn interruption instead of allowing detached work to continue.

The Blaxel runner starts every command as a uniquely named process with a
finite server-side keep-alive timeout, polls the process API for commands that
run beyond the synchronous 60-second window, advertises cancellation, and
force-kills the process group when Roder cancels the command.

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
