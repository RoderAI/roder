## 0.1.3 (2026-07-21)

### Features

#### Bound and cancel detached remote commands

Remote command requests can now carry a wall-clock process lease. Remote shell
and exec tools request provider cancellation when they time out or are dropped
by turn interruption instead of allowing detached work to continue.

The Blaxel runner starts every command as a uniquely named process with a
finite server-side keep-alive timeout, polls the process API for commands that
run beyond the synchronous 60-second window, advertises cancellation, and
force-kills the process group when Roder cancels the command.

## 0.1.2 (2026-06-30)

### Features

#### Blaxel sandbox runner with pause, resume, detach, and rejoin

Replace the placeholder Blaxel runner passthrough with a first-party Blaxel
Sandboxes provider that drives the real control-plane (`/sandboxes`) and
per-sandbox (process/filesystem/preview) REST APIs.

The remote-runner contract gains optional, defaulted lifecycle support so a
runner-bound thread can pause its sandbox toward standby, resume it, fully
detach (releasing the local session while keeping the sandbox alive), and
rejoin the same sandbox from persisted thread state — including across a
process restart, with no orphan sandbox creation. New `RunnerCapabilities`
flags (`pausable`, `detachable`) and `RemoteRunnerSession`/`RemoteRunnerProvider`
methods (`pause`, `resume`, `detach`, `rejoin_session`) default to no-op/false so
existing providers are unchanged.

Exposed through new app-server JSON-RPC methods (`runners/pause`,
`runners/resume`, `runners/detach`, `runners/rejoin`) and a `roder runners` CLI.
The Blaxel credential is sourced from the environment (`BLAXEL_API_KEY` /
`BL_API_KEY`, with `BL_WORKSPACE`) and never written to session state.

A selected runner now actually routes coding tools into the sandbox: a
runtime-level destination (TUI runner picker or config `default_destination`)
auto-binds new threads when the provider advertises a default workspace via the
new `RemoteRunnerProvider::default_workspace` (Blaxel opts in; other providers
are unchanged). Verified live end to end against a real Blaxel account: TUI
shell/file tools execute inside an Alpine sandbox, and pause/resume/detach/rejoin
work through the CLI.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
