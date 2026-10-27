## 0.3.0 (2026-10-27)

### Breaking Changes

#### Release a coherent runtime API compatibility boundary

Adopt explicit shared-read/exclusive/coordinator scheduling, typed custom-provider
capabilities, and owned worktree metadata. Rust integrators must update
ToolExecutor implementations to execution_mode and initialize the new public
configuration and ownership fields.

Release the shared API's reverse dependency closure together so older consumers
cannot resolve patch-compatible packages that expose incompatible API types.
The CLI package also exports a Rust library, so its compatibility release is
0.4.0. Existing command-line use remains supported.

## 0.2.1 (2026-10-25)

### Fixes

#### Preserve token usage across subagent rounds, failures, and interruptions

Configured task agents now accumulate the final usage snapshot from each
inference round instead of reporting only the last round. Runtime turns count
cumulative stream snapshots once and retain observed input, output, cache-read,
cache-write, and service-tier information when a provider fails, a stream ends
with an error, a deadline stops the turn, or the user interrupts execution.

Interrupted turns carry their known usage through persisted history, app-server
notifications, and usage analytics. Thread usage metadata is written before
terminal notifications, and repeated terminal handling cannot charge it twice.
Active-turn native compaction is included once in the same terminal accounting.
Usage that a provider never reports cannot be reconstructed.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
