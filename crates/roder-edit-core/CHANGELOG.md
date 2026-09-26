## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

### Features

#### Align apply_patch parsing, ordered line matching, custom-tool grammar, model routing, and streamed input with Codex. Use one canonical patch argument and Codex patch syntax for local and remote tools. Fix split UTF-8 SSE decoding, terminal completion, client tool-search continuation contracts and exhaustion, replay, pooled HTTP transport, and Retry-After handling. Retry transient sampling failures across runtime profiles, stop on terminal completion, reject silent EOF, preempt sampling on steering, and emit terminal failures once.

Persist completed messages, reasoning, search exchanges and tool effects during sampling. Execute opted-in reads eagerly through normal policy and authorization checks. Add task-scoped WebSocket pooling with verified delta continuation, interruption invalidation and HTTP fallback. Add provider-native, task-aware compaction, complete opaque-window replay, request byte/image guards, failure identifiers, streamed proposed patch progress, actual filesystem diffs and uncertainty-aware partial outcomes and rollback.

Require canonical completion from mock tool responses and keep sampling steering separate from turn cancellation, including immediate-interrupt races. Keep manual compaction and new input atomic for its target task. Prevent concurrent eager reads from deadlocking streamed item persistence. Default new evaluation runs to GPT-6-luna.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
