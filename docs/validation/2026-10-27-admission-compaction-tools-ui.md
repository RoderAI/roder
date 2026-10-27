# Admission, compaction, scheduling, providers, worktrees, and TUI verification

Date: 2026-10-27. Baseline repository HEAD: `44104e3a`.
This verifies the seven-part implementation goal, not every recommendation in
the preceding Codex release audit.

## Requirement-by-requirement evidence

### Prompt admission and recovery

Implementation: `crates/roder-tui/src/app/prompt_admission.rs`,
`prompt_recovery.rs`, and integration in `app.rs` / `app/thread_resume.rs`.

- Pending starts, accepted receipts, unsettled settings/approvals, and copy mode
  hold automatic queue dispatch.
- Each request owns its original payload, images, thread, and correlation ID.
  Definite absent-target steer rejection can fall back once. An uncertain
  response never authorizes automatic resend.
- Rejections arriving out of order preserve submission order. `/retry` preserves
  later queued messages; `/retry discard` explicitly releases a permanently
  rejected input without dropping another thread's input.
- Queue/recovery state survives in-process thread switching and resume. Visible
  button indices map back to the correct underlying thread-scoped queue entry.
- Request and receipt-probe deadlines retain uncertainty rather than claiming
  that an accepted turn did not run. Persisted lifecycle evidence reconciles
  lost events without resurrecting terminal turns.
- Admission-time lifecycle state is not overwritten by a delayed task-start
  event after an interrupt request.

Evidence: 14 admission/recovery regressions in `prompt_admission_tests.rs` and
`prompt_recovery_tests.rs`; app-server
`turn_interrupt_reports_durable_lifecycle_state`; live mock-provider TUI run.
The queue is not a durable process-restart outbox; see `docs/prompt-admission.md`.

### Transactional compaction: explicit and automatic

Implementation: `crates/roder-core/src/provider_compaction.rs`,
`runtime/turn_compaction.rs`, `compaction.rs`, and the Responses compaction/replay
modules.

- Explicit compaction stages opaque candidates until validated terminal success.
- Ordinary streamed compaction follows the same invariant, including
  `compacted_input` metadata without an `output` field.
- Failure, disconnect, retry, and steering discard candidates without replacing
  canonical replay history. Observed usage remains accounted independently.
- Successful automatic compaction stores its boundary and retained current-round
  output/eager results in one commit envelope. Replay expands the suffix once
  and strips internal fields.
- Invalid/nested envelopes are quarantined, not left available to the provider's
  separate replay-trimming path.
- Commit metrics and boundary effects occur before published successful
  completion. Ordinary responses without pending compaction retain their normal
  completion timing.

Evidence: explicit transaction/restart/billing tests in
`provider_compaction_transaction_tests.rs` and
`native_compaction_transaction_tests.rs`; six automatic-compaction regressions
in `runtime/turn_compaction_tests.rs` and `turn_compaction_wire_tests.rs`.

The real HTTP/SSE runtime fixture completes an eager read before releasing
`response.completed`. It checks the next request and cold-restart replay for
exactly one opaque boundary, message, call, and result, with call-before-result
ordering, for ordinary and explicit-window responses. It also verifies no
internal commit marker reaches the wire.

The existing local-summary path was inspected: `compaction_runtime.rs` requires
terminal completion before accepting generated summary text.

### Safe tool scheduling

Implementation: `roder-api/src/tools/scheduling.rs`,
`roder-core/src/tool_scheduling.rs`, runtime eager/normal dispatch, MCP and
subagent dispatchers.

- Explicit `SharedRead`, default `Exclusive`, and audited `Coordinator` modes.
- Adjacent reads can overlap; mutations cannot be overtaken by subsequent reads.
- Normal, eager, provider-driven, and nested task/swarm execution share the
  runtime barrier without recursively locking coordinators.
- MCP read hints require an exact local trusted-tool allowlist and never replace
  authorization, approvals, or authentication.
- Remote session mutexes and provider workspace fences remain intact.

Evidence: barrier overlap/exclusion/cancellation/policy regressions, eager
ordering tests, nested subagent tests, and existing/new remote serialization,
provider-fencing, and lost-lease tests. See `docs/tool-scheduling.md`.

### Custom Responses-provider capabilities

Implementation: typed API capabilities propagated through configuration, CLI,
extension host, and Responses engine.

```toml
[providers.example.capabilities]
remote_compaction = "v2"
external_web_access = false
```

Custom providers can opt into native V2 compaction and cap hosted web access.
Supporting native compaction is separate from requiring native compaction.
Omitted fields preserve existing defaults; per-turn settings cannot exceed the
provider's hosted-web ceiling.

Evidence: configuration round trips, CLI/host propagation tests, and ordinary
request plus native-compaction HTTP fixtures. See
`docs/custom-responses-providers.md`.

### Source-copy UX

Implementation: `transcript/source_copy.rs`, `app/source_copy.rs`, timeline
source identities, and live `/copy`/keyboard/clipboard integration.

Whole-response and fenced-block payloads come from original source byte ranges,
not visual rows. Navigation and selection survive streaming and resizing.
Clipboard success waits for subprocess completion; failure retains the selected
source and displays an error. Origin-thread/selection checks prevent stale
completions from closing another selection.

Evidence: source-copy regressions for whitespace, CRLF/mixed endings, fences,
Unicode preview wrapping, identity retention, and delayed clipboard outcomes.
Live TUI verification opened `/copy`, displayed a real missing-clipboard error,
closed it and accepted another prompt. A second isolated run substituted a
test-only `wl-copy` executable and asserted that the production clipboard
subprocess received exactly `b'hello from roder'`, without touching the real
clipboard. See `docs/tui-source-copy.md`.

### Owned agent worktrees

Implementation: `spawn_agent` workspace isolation, core agent/fork integration,
Git ownership records, and existing public fork endpoints.

`workspace_isolation: "worktree"` routes child coding tools to a durable,
owned checkout independently of conversation-history forking. Same-owner retries
reuse the checkout. External registered worktrees cannot be adopted or removed.
Cleanup remains explicit, path-confirmed, non-force, and blocked for active
owners or dirty checkouts. Failed starts only attempt safe owned-checkout cleanup.

Evidence: two real child tool executions write into separate workspaces;
ownership/metadata restart tests; external-worktree rejection, duplicate
creation, dirty-source/cleanup, active-owner, remote-parent and rollback tests;
public JSON-RPC fork lifecycle tests.

This is workflow isolation, not an OS security sandbox. Existing path
permissions remain authoritative. See `docs/app-server/agent-worktrees.md`.

### Terminal image previews

Implementation: `app/media/`, timeline layout hooks, worker polling, and
`app/preview_layout.rs`.

- Actual Kitty/Sixel encodings and reserved transcript rows, not preview labels.
- Conservative detection, bounded asynchronous decoding/encoding, and metadata
  fallback for unsupported terminals and invalid images.
- Local view paths resolve against authoritative thread cwd. Remote/unknown
  view paths never become host filesystem reads. Artifact storage paths must be
  absolute.
- Pixel placements are clipped to transcript bounds, suspended under overlays,
  and cleaned up without deleting other programs' Kitty images.
- Image escape sequences do not enter copyable source.

Evidence: 24 media tests and two timeline-origin tests, including actual protocol
payloads, both-edge clipping/composer isolation, bounds/corruption, stale-width
completion, idle redraw, overlay suspension, origin retargeting, and cleanup.
Interactive physical Kitty/Sixel display was not observed: the environment is
headless tmux with no graphical display. Protocol/layout tests are the pixel
validation evidence; pane captures are not presented as pixel-protocol proof.

## Final frozen-source gates

| Gate | Result |
| --- | --- |
| Full core + Responses suites | 629 passed, 0 failed, 3 explicitly live tests ignored |
| Full TUI suite | 669 passed, 0 failed |
| App-server E2E suite | 136 passed, 0 failed, 1 explicitly live timer test ignored |
| CLI capability propagation test | 1 passed |
| CLI build | Passed |
| Formatting | `make fmt-check` passed |
| Local changeset coverage | All 14 affected released packages covered; keys match `knope.toml` |
| Knope generation and whitespace | Checks passed |

Additional component gates passed: config 88, MCP 23, subagents 19, extension
host 76, tools 134, full Git 33, full API suite and public fork JSON-RPC tests.

Final commands/logs:

```text
mise exec -- cargo test -p roder-core -p roder-ext-openai-responses -j4 -- --test-threads=1
/tmp/roder-goal-final-frozen-core-responses.log

mise exec -- cargo test -p roder-tui -j4
/tmp/roder-goal-final-frozen-tui.log

env -u CURSOR_API_KEY -u RODER_CURSOR_API_KEY mise exec -- cargo test \
  -p roder-app-server --features e2e-tests --test e2e -j4 -- --test-threads=1
/tmp/roder-goal-final-frozen-server-e2e.log

mise exec -- cargo test -p roder --bin roder custom_provider_tests -j4
mise exec -- cargo build -p roder --bin roder -j4
mise exec -- make fmt-check
```

App-server tests were serialized because they modify process-global configuration
environment. Existing Cursor credentials were removed only from the test
process, not the user's environment, to make the unauthenticated-state fixture
valid.

Strict Clippy was run and encounters existing warnings in API/web-search and
unrelated TUI code. TUI all-target/no-dependency lint passed with only the known
`large_enum_variant`, `manual_div_ceil`, `unnecessary_lazy_evaluations`,
`type_complexity`, and `assertions_on_constants` categories allowed on the
command line. No source lint suppressions were added. The clean scoped run is
not claimed to be a clean strict workspace run.

One stale context-size recovery fixture was corrected after pristine HEAD
reproduced its failure. Production recovery behavior was not weakened; genuine
terminal errors retain exact-one-request assertions and size failures now test
the existing bounded recovery contract.

Independent read-only reviews found and closed the ordinary-stream and
no-output-window compaction bypasses. Final delta review found no remaining
blocker in the seven requested areas.

## Live TUI artifacts

Text captures and rasterized pane-text PNGs:

```text
.roder/tmux-captures/goal-admission-copy/
  roder-goal-validation-20261026-233814.{txt,png}  copy selection
  roder-goal-validation-20261026-234057.{txt,png}  actual clipboard failure
  roder-goal-validation-20261026-234058.{txt,png}  subsequent prompt accepted
  roder-goal-clipboard-20261027-001012.{txt,png}   isolated clipboard success
```

Both task-created tmux sessions were stopped. User configuration and real
clipboard contents were not changed. Unrelated in-flight work was preserved.
