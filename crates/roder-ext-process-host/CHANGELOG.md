## 0.1.3 (2026-08-05)

### Fixes

#### Add Ultra mode as a first-class multi-agent mode for any model

Make Codex Ultra's proactive multi-agent policy a concrete Roder mode
(`/ultra`, `thread/set_ultra_mode`, `settings/get.ultraMode`,
`ultra/modeChanged`), available for every model — not only Sol/Terra Ultra
reasoning effort. Sol/Terra Ultra effort still maps to max wire effort and
enables proactive multi-agent without requiring the mode flag.

Also: `task` / `agent_swarm` children inherit the parent thread's live
provider+model (so SuperGrok stays on grok-4.5), and lane `max_concurrent`
can be raised per request for large fanouts instead of hard-failing at the
old scout cap of 4.

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

### Features

#### One-command Roder package install (`roder install npm:/git:/path`)

Roder packages bundle process extensions, skills, slash commands, and themes
behind a root `roder.toml` manifest. Install from npm, git (shorthand, SSH,
raw URLs, pinned refs), or local paths; manage with `roder packages
list|resources|enable|disable|approve|filter|sync|init`, `roder remove`,
`roder update`, and ephemeral `-e` loading. Resources surface through the
existing skills/commands/theme registries; the process-extension protocol
gains manifest-declared tool providers served over `tools/call`. New
app-server `packages/*` methods, a `/packages` builtin, and a Packages
palette section round out the surfaces. npm lifecycle scripts stay disabled
unless `--allow-scripts` is passed, and package process extensions never
launch before explicit approval.

### Fixes

#### Process-extension protocol 0.2.0 and Cursor SDK remote-agent bridging

Extend the process-extension protocol with subagent-dispatcher and task-executor services, bridge them in the process host, and add app-server e2e coverage for the cursor-sdk-agents TypeScript child.

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
