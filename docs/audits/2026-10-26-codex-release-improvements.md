# Codex release improvements worth bringing to Roder

Audit date: 2026-10-26, America/Los_Angeles. Release timestamps below are UTC.
Scope: official Codex releases 0.160.0 through 0.164.0, plus a separately
labelled observation from newer main. This is an audit, not an implementation.

Implementation follow-up: the seven selected areas are covered by
`docs/validation/2026-10-27-admission-compaction-tools-ui.md`. The source findings
below describe the pre-implementation audit state; other recommendations such
as credential redaction, patch line endings, and inline model flags remain
separate suggestions, not part of that implementation goal.

## Evidence and limitations

- Official GitHub release metadata was queried directly. Its `releases/latest`
  endpoint returned `rust-v0.164.0`, published 2026-10-23 at 04:48 UTC.
  The 0.165.x and 0.166.x releases observed were prereleases.
- Codex source: `/home/pz/w/codex`, HEAD `ff2346f33f851bd4e90c8e42ae60abcc7e7f293b`.
  Its timestamp is 2026-10-27 UTC, still October 26 locally.
- Roder source: HEAD `44104e3a`, plus the working tree inspected during the
  audit. Other work changed TUI files during inspection; symbol names are
  more durable anchors than their approximate line numbers.
- Release-linked commits were inspected locally. Key compaction, provider
  capability, patch, queue, scheduler, and pagination implementations were
  additionally checked against the public `rust-v0.164.0` source tag.
  The checkout does not contain that stable tag locally.
- Findings are static source observations, not reproduced production incidents.
  No builds, runtime experiments, live provider requests, or regression tests
  were run. Suggested tests below are acceptance criteria, not passing results.

## Executive recommendation

Do not clone Codex's feature list. Roder already has native compaction, live
steering, editable prompt queues, parallel tools, searchable settings, reusable
commands, and provider-backed worktree forks. The best opportunities are
stronger invariants around those features, plus a few genuinely useful UX gaps.

Priority P1 means security/correctness; P2 means capability or UX improvement.
Effort estimates are relative, not delivery commitments.

| Rank | Improvement | Priority | Effort |
| --- | --- | --- | --- |
| 1 | Redact actual credential values in hosted-runner errors | P1 | Small |
| 2 | Commit compaction history only after complete validation | P1 | Medium |
| 3 | Make queued and steered prompts acceptance-aware | P1 | Medium |
| 4 | Preserve existing line endings on the default patch path | P1 | Small |
| 5 | Unify shared-read/exclusive-mutation tool scheduling | P1 | Medium |
| 6 | Configure capabilities for custom Responses providers | P2 | Medium |
| 7 | Add keyboard copy mode backed by source, not visual rows | P2 | Medium |
| 8 | Expose owned worktree isolation to coding agents | P2 | Medium |
| 9 | Render actual terminal image previews | P2 | Medium-large |
| 10 | Extend inline model selection with validated flags | P2 | Small |

Suggested first batch: 1 and 4 for small, high-confidence wins; 2 and 3 next
for long-session reliability. Treat 5 as a deliberate scheduling design change,
not a broad "make everything parallel" optimization.

## 1. Redact credential values, not credential field names

**Codex inspiration:** 0.164.0, #54309, #54432, #56049: credential masking for
supported remote shell operations and custom-provider credentials.

**Concrete Roder gap:** `crates/roder-ext-runner-hosted-common/src/lib.rs:440`,
`redact_body`, replaces the words `token`, `api_key`, `secret`, and
`authorization`. It does not remove the associated values. For example:

```text
input:  {"token":"SENTINEL-7f9c"}
output: {"<redacted>":"SENTINEL-7f9c"}
```

`decode_response`, `decode_bytes`, and `decode_empty` interpolate this body into
errors at lines 395-420. Thus an endpoint that echoes a credential in an error
can defeat the intended sanitization. This is not evidence of an actual leak.

Existing config redaction at lines 425-435 handles top-level values, and the
known token is available in runner configuration/session state. The current
body-redaction test at lines 669-677 checks disappearance of marker words,
not disappearance of a marker-free secret.

**Change:** Carry known secrets into error sanitization; redact sensitive JSON
values recursively with case-insensitive key handling. For unstructured bodies
that cannot be reliably sanitized, prefer a bounded generic error with status
and a diagnostic fingerprint over returning raw contents.

**Acceptance:** Known token never appears in errors for nested JSON, differently
cased keys, bearer strings, escaped values, or non-JSON responses. Harmless
diagnostics remain useful. Test synthetic secret values, not only field names.

This is a focused boundary fix. Codex's full credential proxy and executor-local
shell capture are architectural references, not a cheap drop-in dependency.

## 2. Make compaction transactional with respect to canonical history

**Codex inspiration:** 0.164.0, #53932, commit `400f2356d6`: failed compaction
must not leave partial summaries in history. Codex's
`codex-rs/core/src/compact.rs`, `drain_to_completed`, stages output until
`ResponseEvent::Completed`. Its opaque V2 path in `compact_remote_v2.rs`
also requires terminal completion and valid compaction output.

**Roder's actual policy is different, intentionally:**

- `crates/roder-ext-openai-responses/src/native_compaction.rs:114-123`,
  `compaction_stream`, emits boundary metadata when an individual opaque item
  finishes, before response completion.
- `crates/roder-core/src/provider_compaction.rs:99-105`,
  `compact_with_provider`, persists it immediately and replaces the retry
  transcript.
- Later disconnect, explicit failure, conflicting boundary, or executable tool
  call can still fail the operation.
- `crates/roder-core/src/compaction.rs:70-84` treats the persisted boundary as
  authoritative on replay.

Consequently, a failed operation can still change the next turn's canonical
context. This is a policy decision to revisit, not missing native compaction
and not proof that every individually completed opaque item is invalid.
`docs/audits/2026-10-08-openai-compaction.md:131-135` documents the early-durability
decision. Existing tests explicitly enforce it:
`codex_compaction_boundary_is_durable_before_disconnect` and
`manual_native_compaction_uses_thread_selection_and_retries_committed_window`.

**Change:** Stage history metadata per attempt; commit only after response
completion and full protocol validation. If recovery candidates must survive a
crash, persist them as uncommitted state that replay cannot automatically adopt.
Continue accounting for usage from failed attempts.

**Acceptance:** Boundary then disconnect/failure/invalid second item leaves
replay evidence unchanged, including after cold restart and exhausted retries.
Steering preemption does not commit a partial attempt. Successful compaction
commits once and remains resumable.

**Provenance:** The newly released fix concerns local-summary buffering. The
opaque V2 completion invariant predates it; the recommendation applies the
same invariant to Roder's deliberately different persistence policy.

## 3. Make prompt admission and queue recovery acceptance-aware

**Codex inspiration:** 0.164.0 #55682, 0.160.0 #49105, and the 0.163.0
instant-steering improvements #51812/#51835.

Roder already preempts inference for steering and supports editing, deleting,
and steering queued prompts. The missing improvement is reliable admission
and per-message recovery.

**Roder evidence**, all in `crates/roder-tui/src/app.rs`:

- `should_queue_prepared_prompt` only checks `active_turn_id`. A `turn/start`
  can already be outstanding before `TurnStarted` establishes that ID.
- `start_prepared_prompt` records a pending request but does not establish an
  admission gate for the pending-start window.
- The `TurnCompleted` handler dispatches the next queued prompt without a
  centralized unresolved-settings check.
- `submit_next_queued_prompt` and `steer_queued_prompt` remove queue entries
  before backend acceptance.
- `PendingTurnRequest` does not own the submitted payload; recovery relies on
  one `last_user_prompt`.
- `run_retry_slash_command` clears the remaining queue.

Codex's `chatwidget.rs` has an explicit `permission_popup_pending` state.
`chatwidget/input_flow.rs`, `maybe_send_next_queued_input`, centralizes the
admission decision; `input_queue.rs` distinguishes queued, pending-start,
pending-steer, and rejected input.

**Change:** Introduce one prompt-admission state machine. Pending requests own
the exact prompt, attachment references, origin, and client message ID.
Block dispatch during outstanding starts and unresolved setting selections.
Commit on acceptance; restore only the rejected prompt on definite rejection.
Do not automatically resend ambiguous transport failures without idempotency.
Keep the existing protection against clearing a newer active turn.

**Acceptance:** Two submissions before `TurnStarted` yield one start request.
Queue pauses while settings are unsettled and resumes once on accept/cancel.
Reject steer A while accepting B: A is recoverable verbatim and queued C/D
remain intact. A completion/steer race neither duplicates input nor loses it.
Backend rejection and stale UI requests cannot leave a permanent hold.

Codex's current `app/user_turn_submission.rs` is useful additional race-recovery
source, but not all of that implementation is newly attributable to #51812.

## 4. Make line-ending preservation the canonical patch behavior

**Codex inspiration:** 0.162.0 #51203, commit `685270a56a`.
`codex-rs/apply-patch/src/file_update.rs:45-50` parses the source's line endings
and renders preserved contents without requiring an opt-in.

**Roder evidence:** `crates/roder-edit-core/src/patch.rs:18-23` already offers
`PreserveLineEndings`, but `NormalizeToLf` is the default. Both local
`patch/filesystem.rs:59,115` and remote
`crates/roder-tools/src/patch_runner.rs:51,111` use that default.
The mixed-ending test in `patch/tests.rs:121-137` exercises explicit
preservation, not the production default.

**Change:** Preserve existing terminators by default in local and remote edits.
Prefer a single canonical update behavior over retaining obsolete opt-in modes.

**Acceptance:** Production entry-point tests for CRLF, mixed LF/CRLF/CR,
untouched context bytes, EOF insertion, missing final newline, and rename plus
update. Assert byte-level locality, not just logical-line equality.

## 5. Unify safe read parallelism and mutation barriers

**Codex inspiration:** 0.163.0 #52245. Its released
`codex-rs/core/src/tools/parallel.rs:213-216` grants parallel-safe handlers a
shared read lock and other handlers an exclusive lock.

**Roder already parallelizes tools:** `crates/roder-core/src/runtime.rs`,
`route_tool_calls`, uses `try_join_all` when parallel execution is enabled,
excluding agent/event controls. `crates/roder-api/src/tools.rs:323-327` also has conservative
`supports_eager_execution` opt-ins.

The real gaps are consistency and classification:

- Local batch scheduling has no general shared-read/exclusive-mutation
  distinction.
- `crates/roder-ext-mcp/src/client.rs:14-20` drops tool annotations, and
  `tool.rs:60-75` does not opt MCP tools into eager execution.
- Remote execution deliberately takes the shared session mutex and a provider
  workspace lease in `crates/roder-core/src/tool_execution.rs:530-575`.
  Those fences protect compound read-modify-write operations.

**Change:** Add conservative scheduling metadata with unknown/mutating tools
exclusive by default. Use it for normal and eager execution. Treat MCP
annotations as untrusted hints requiring local trust/configuration, never as
authorization. Only relax remote read serialization when the provider supports
shared read leases.

**Acceptance:** Independent reads overlap; mutations cannot overlap relevant
reads or mutations. Unknown MCP tools stay exclusive. An annotation cannot
bypass approvals. Remote compound edits retain their full fencing lease.

## 6. Allow custom Responses providers to declare capabilities

**Codex inspiration:** 0.162.0 #50459, commit `c6049ebd96`.
`codex-rs/model-provider-info/src/capabilities.rs` provides optional
`external_web_access` and `remote_compaction = unsupported|v2`.

**Roder evidence:**

- `crates/roder-config/src/lib.rs`, `ProviderConfig`, has no equivalent fields.
- `crates/roder-extension-host/src/lib.rs`, `CustomInferenceProviderConfig`
  and its registration path, pass ID/name/key/base URL, not capabilities.
- `crates/roder-ext-openai-responses/src/provider.rs:803-805`,
  `requires_native_compaction`, is hardcoded to `openai` or `codex`.
- `native_compaction.rs`, `custom_responses_provider_keeps_local_compaction_fallback`,
  explicitly tests local fallback for custom providers.
- `tool_definitions.rs:10-24` derives hosted web access from the requested mode,
  without a custom-provider capability ceiling.

**Change:** Propagate typed provider capabilities through config, host, and
engine. Preserve current defaults; let compatible providers explicitly opt
into V2 compaction and restrict external search. Keep "supports native
compaction" distinct from "requires native compaction."

**Acceptance:** Config round-trip; omitted fields retain behavior; custom V2
emits a valid trigger and replays opaque history; unsupported stays local;
turn settings cannot exceed provider web-access policy.

## 7. Copy source blocks using the keyboard

**Codex inspiration:** 0.162.0 #50434; see
`codex-rs/tui/src/transcript_view/copy_mode.rs`.

Roder already has mouse selection, clipboard infrastructure, context-menu
copy, and keybinding definitions. In `crates/roder-tui/src/app.rs`,
`selected_text` joins rendered visual rows with newlines and trims trailing
whitespace. `handle_mouse_selection` reports success without checking the
clipboard result. The inspected built-in command catalog has no `/copy` mode.

**Change:** Add response/code-block keyboard navigation with copy payloads
backed by original transcript source rather than terminal-wrap rows.
Keep highlighting separate from copied bytes and surface clipboard failures.

**Acceptance:** Wrapped code copies without artificial newlines, prefixes,
line numbers, or lost whitespace. Navigation survives resize/streaming.
Test whole responses, code blocks, lists, tables, quotes, and line endings.
Leaving copy mode releases any deliberate prompt-admission hold.

## 8. Give coding agents owned worktree isolation

**Codex inspiration:** 0.162.0 #50148, commit `e7ea5f4a86`.
`managed_worktree_tool_specs.rs` and `managed_worktree_tools.rs` expose deferred
create/status/list operations with explicit base-ref and ownership semantics.

**Roder already has the backend:** `crates/roder-core/src/forks.rs` and
`crates/roder-app-server/src/forks.rs` provide create/list/resume/remove.
`crates/roder-ext-git/src/worktree.rs` rejects dirty sources, creates from HEAD,
rejects primary/unregistered removals, and uses non-force Git removal.

**Specific opportunities:**

- `crates/roder-core/src/agent_control_tools/spec.rs` exposes context-history
  forking, not a workspace-isolation selection for spawned agents.
- `crates/roder-ext-git/src/fork_provider.rs`, `list_forks`, includes all
  registered non-primary worktrees, not just Roder-created ones.
  Exact-path confirmation does not establish ownership.

**Change:** Reuse `ForkProvider` for opt-in agent isolation or deferred
model-facing worktree tools. Persist creator/thread ownership first, and
distinguish external worktrees. Keep deletion separately confirmed.
Explicit base-ref selection is useful; retain today's strict dirty-source
default unless an explicit policy says changes will not be copied.

**Acceptance:** Two isolated coding agents cannot overwrite each other's
workspace. Duplicate creates converge on one pending operation. Unrelated
registered worktrees are not adopted or cleaned up. Dirty-source refusal,
base-ref validation, and non-force removal remain intact.

## 9. Render real image previews in supported terminals

**Codex inspiration:** 0.164.0 #54216; see
`codex-rs/tui/src/transcript_image.rs`.

**Roder evidence:** `crates/roder-tui/src/app/media.rs`,
`MediaTimelineRow::render`, produces metadata lines; `InlineImage` becomes the
literal label "inline preview." Image inputs and attachment chips already
exist, but the inspected TUI has no Kitty/Sixel pixel-rendering path.

**Change:** Capability-detected Kitty/Sixel previews for local screenshots and
generated images first, with bounded decoding and metadata/open-file fallback.
Integrate image height into transcript layout, not copied/exported text.

**Acceptance:** Unsupported terminals, tmux, resizing, scrolling, corrupt
images, decompression bombs, and lifecycle cleanup. Images must not cover the
composer or leave stale placements. Text-only transcript behavior stays useful.

## 10. Extend existing inline model selection, rather than duplicate it

**Codex inspiration:** 0.164.0 #54378, commit `0884c8047a`; see
`codex-rs/tui/src/chatwidget/model_command.rs`.

**Roder evidence:** `crates/roder-tui/src/app.rs`,
`run_model_slash_command`, already accepts inline model IDs. It treats the
entire argument as the ID and selects with `reasoning: None`.
The model/reasoning palette already supplies suitable selection metadata.

**Change:** Parse optional `--reasoning` and `--speed`, including flags-only
updates. Validate the complete change before applying it; route it through
the existing routing-aware model-selection API.

**Acceptance:** Space-separated and equals forms; model-specific allowed
values; invalid input changes nothing; explicit current-thread/default scope;
no accidental pinning of an automatically routed default model.

## Additional infrastructure opportunities

### Paginate transcript history, not just the thread list

This is supported by the released 0.164.0 source and changelog work such as
#54351 (turn-scoped item pagination), but is a larger platform investment
than the quick wins above.

Codex's released protocol defines `ThreadTurnsListParams` and
`ThreadItemsListParams` in `app-server-protocol/src/protocol/v2/thread.rs`.
Roder's `crates/roder-protocol/src/lib.rs:709`, `ThreadReadParams`, offers
only `include_turns`; `handle_thread_read` in
`crates/roder-app-server/src/server.rs` loads and projects the full snapshot
when true. Thread-list pagination does not solve that payload/latency problem.

Add bounded turn/item reads with stable opaque cursors and store-level indexes.
Do not merely slice a fully loaded snapshot. Test streaming concurrent with
pagination, anchored reads after appends, unavailable/corrupt histories,
bounded memory, and reconnect reconciliation.

### Cross-process thread-index coordination: newer-main watchlist

**Not attributed to stable 0.164.0.** Local Codex HEAD includes #57358,
commit `adfcac1a42`, timestamped 2026-10-27 UTC. It protects session-index
mutations with a persistent OS lock, retains the lock across cancellation,
and bounds contention. #57360 similarly bounds writer coordination waits.

Roder's `crates/roder-ext-jsonl-thread-store/src/store.rs:64-103` already
uses unique temporary files and atomic rename. However,
`upsert_thread_index` and `remove_thread_index_entry` separately load,
modify, and replace the whole index without cross-process coordination.
Two writers can read the same starting state and the last rename can erase
the other's index entry. A valid but incomplete index is not automatically
rescanned merely because a thread is absent from it.

This is a static lost-update risk, not a reproduced incident. Introduce bounded
cross-process coordination around the entire read/modify/replace operation,
including rebuilds, with lock ownership surviving cancellation of async callers.
Do not infer that atomic rename makes that transaction atomic.
Test multiple processes, cancelled waiters, bounded timeouts without writes,
rebuild/update races, and visibility after restart.

## Deliberately excluded or narrowed

- **"Add steering":** already present in `runtime/sampling.rs` and the TUI.
- **"Add parallel tools":** already present; recommendation 5 is about safety
  and consistent classification.
- **"Add worktrees":** already present; recommendation 8 is exposure/ownership.
- **"Add searchable settings":** the existing Ctrl+P settings palette covers it.
- **"Add presets":** existing custom commands already cover reusable prompts.
  A new surface should add real quick-recall or settings-bundle value.
- **"Honor Retry-After":** the Responses transport already does this, with
  `audit_http_retry_honors_retry_after` regression coverage.
- **"Restore service tier on resume":** Roder already persists and reads it,
  with explicit per-turn override precedence. No equivalent #55923 loss
  was established.
- **"Fix background compaction receiver panic":** Roder's inspected path awaits
  compaction inline; no equivalent #55581 receiver lifecycle was established.
- **Proxy DNS, duplicate Host, sandbox case aliases, executable symlinks:**
  useful regression ideas, but no equivalent Roder enforcement path was proven.
  They should not be presented as confirmed directly portable security bugs.

## Official release sources

Fetched from GitHub's official `openai/codex` repository API, not a third-party
summary. Release dates:

| Version | Published UTC |
| --- | --- |
| 0.160.0 | 2026-10-01 20:19 |
| 0.161.0 | 2026-10-07 15:58 |
| 0.162.0 | 2026-10-08 18:55 |
| 0.163.0 | 2026-10-15 21:30 |
| 0.164.0 | 2026-10-23 04:48 |

```text
https://api.github.com/repos/openai/codex/releases/latest
https://github.com/openai/codex/releases/tag/rust-v0.160.0
https://github.com/openai/codex/releases/tag/rust-v0.161.0
https://github.com/openai/codex/releases/tag/rust-v0.162.0
https://github.com/openai/codex/releases/tag/rust-v0.163.0
https://github.com/openai/codex/releases/tag/rust-v0.164.0
```
