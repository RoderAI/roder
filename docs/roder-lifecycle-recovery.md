# Roder Lifecycle Recovery

Roder records a small, durable lifecycle history for each turn. The history
answers a narrow but important question after interruption, shutdown, or a
restart: did the turn finish, was cleanup requested, or does a user need to
inspect and recover it?

## Inspect a thread

Use the CLI for a concise local view:

```sh
roder thread lifecycle <thread-id>
roder thread lifecycle <thread-id> --json
```

App-server clients can call `thread/read` with `includeTurns: false` and inspect
its additive `lifecycle` field. The TUI shows a durable warning when a resumed
thread contains `recovery_needed` records or redacted lifecycle-corruption
diagnostics.

Each record contains a lifecycle `state`, a `cleanup` result, a redacted
`ownership` proof level, an optional reason, and a timestamp. The latest valid
record for a turn is authoritative.

## State and cleanup meanings

- `running` and `interrupt_requested` are nonterminal records. If Roder restarts
  while no matching turn remains local, it reconciles either state to
  `recovery_needed` with reason `runtime_restart`.
- `interrupted` means the runtime accepted an interruption path. It is not the
  same as a completed model response.
- `completed` and `failed` mean the turn reached its normal terminal runtime
  path. They do not disclose provider command details.
- `recovery_needed` means Roder cannot safely present a stale active turn as
  completed. Inspect the transcript and lifecycle record before explicitly
  resuming or retrying safe work.

Cleanup values are `not_requested`, `requested`, `completed`, `timed_out`, and
`unknown`. `completed` only describes the cleanup path represented by the
lifecycle record; use `ownership` to understand the available proof:

- `runtime_task_only`: Roder observed only its own async turn task. A provider
  child or remote job has not supplied a reaping acknowledgement.
- `provider_cleanup_pending`: the provider registered owned cleanup, but Roder
  has not observed completion yet.
- `provider_cleanup_confirmed`: the provider reported that its owned cleanup
  path completed. This is deliberately redacted: it does not expose PIDs,
  command lines, credentials, or remote job identifiers.

## Execution ownership boundaries

Roder reports the narrowest proof it has for each execution owner:

- Ordinary inference streams are `runtime_task_only` unless the provider
  registers a `ProviderTurnCleanup` acknowledgement. Interrupting the Roder
  task does not by itself prove that an arbitrary provider-owned child or remote
  job exited.
- Claude Code registers provider cleanup through the vendored SDK supervision
  path. Its offline fake-CLI regression proves that the child is reaped before
  Roder records `provider_cleanup_confirmed`.
- Local process tasks run in a child-owned Unix process group where supported.
  Roder sends `SIGTERM`, waits for the configured grace period, escalates to
  `SIGKILL` when needed, waits for `child.wait()`, and only then marks the
  registry descriptor stopped. Children that deliberately escape the owned
  process group remain outside this guarantee.
- Remote process tasks never use host PIDs. Their stop path calls the selected
  remote runner session's cancellation API; a remote cancellation failure stays
  visible as a nonterminal process descriptor.
- Background subagent and process-host tasks use their executor cancellation
  surfaces, but generic executors that do not provide a reaping acknowledgement
  remain `runtime_task_only`.
- Local `command/exec` children are Roder-owned and use the process registry,
  but their lifecycle proof is separate from an inference provider cleanup
  acknowledgement.

## Shutdown and drain

Before a local TUI or app-server exits, Roder can run `runtime/drain`. The drain
stops admitting new turns, requests interruption for locally owned work, and
waits within a bounded deadline for runtime and app-server-owned task/process
cleanup. Its result is one of:

- `clean`: the runtime work selected by the drain reached its cleanup path and
  no app-server-owned process remained registered. Providers without explicit
  ownership acknowledgement are still represented as `runtime_task_only`.
- `deadline_exceeded`: local runtime, task, or process work remained when the
  deadline elapsed.
- `persistence_failed`: at least one lifecycle write failed during the drain.

The canonical shutdown configuration defaults to five seconds:

```toml
[lifecycle]
shutdown_drain_timeout_ms = 5000
process_grace_timeout_ms = 250
process_kill_timeout_ms = 1000
cancel_tasks_on_session_end = true
max_completed_process_diagnostics = 64
```

The equivalent environment variables are
`RODER_LIFECYCLE_SHUTDOWN_DRAIN_TIMEOUT_MS`,
`RODER_LIFECYCLE_PROCESS_GRACE_TIMEOUT_MS`, and
`RODER_LIFECYCLE_PROCESS_KILL_TIMEOUT_MS`, and
`RODER_LIFECYCLE_CANCEL_TASKS_ON_SESSION_END`, and
`RODER_LIFECYCLE_MAX_COMPLETED_PROCESS_DIAGNOSTICS`. The terminal process
diagnostic limit is clamped to 1024 and bounds completed process descriptors and
their bounded output tails in the in-memory process registry. It does not prune
durable `roder.lifecycle` recovery records. The legacy
`[tui].shutdown_drain_timeout_ms` and `RODER_SHUTDOWN_DRAIN_TIMEOUT_MS` remain
fallbacks only when the canonical lifecycle drain setting is absent. The
explicit app-server `runtime/drain` request accepts `timeoutMs` and clamps it to
1 through 45000 milliseconds.

The CLI retains a final hard `std::process::exit(0)` fallback after its bounded
drain. This is a terminal-restoration safeguard, not proof that every
third-party provider has reaped external work. Roder documents limitations in
the lifecycle record rather than silently treating interrupted work as
completed.

## Metrics

Native app-server clients can call `lifecycle/metrics` with `{}` for fixed,
redacted, process-local counters. It reports drain outcomes and total drain
duration, lifecycle persistence failures, restart reconciliation, and provider
cleanup confirmation, timeout, or unknown-proof counts. The response has no
provider labels, PIDs, process IDs, command lines, thread IDs, turn IDs,
prompts, or credentials.

The counters help detect lifecycle trends, but they are not proof for an
individual turn. Use `thread/read.lifecycle` or `turn/lifecycleUpdated` and the
record's `ownership` field when deciding whether a particular provider child or
remote job has acknowledged cleanup.

## Persistence and corruption

Lifecycle records use the versioned `roder.lifecycle` extension state. They are
append-only; this permits restart reconciliation without rewriting transcript
history. A lifecycle record write is atomic at its own store append/insert
boundary, but a lifecycle record and a separate event/transcript write are not
a cross-store transaction.

The JSONL thread store skips malformed lifecycle extension lines, preserves the
thread when its other data is valid, and reports only a redacted
`corruptRecordCount`. PostgreSQL stores the same extension-state schema, skips
malformed extension-state rows during thread load, and adds the same redacted
corruption marker. Both stores preserve valid records for unrelated turns. The
PostgreSQL lifecycle persistence/restart regression remains opt-in because it
requires `RODER_POSTGRES_SESSION_TEST_URL`.

## Claude Code and ACP

The Claude Code provider uses a narrowly vendored SDK patch with receiver-close
handling, `kill_on_drop(true)`, and disconnect/reap behavior. Offline fake-CLI
tests cover both the SDK and Roder provider interrupt path. The opt-in live
Claude cancellation smoke remains separate because it requires local
authentication.

ACP `session/cancel` maps to standard cancelled prompt completion. Durable
lifecycle records, recovery diagnostics, `runtime/drain`, `lifecycle/metrics`,
and ownership proof levels are Roder native app-server features; ACP clients
that need those details must use the native JSON-RPC surface.
