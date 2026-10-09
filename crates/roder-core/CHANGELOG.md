## 0.3.6 (2026-10-09)

### Features

#### Native Linux and macOS desktop computer use through Cua Driver

Enable runner-bound Cua desktop tools with observation, grounded pointer and
keyboard input, Unicode value replacement, window controls, permission gating and fresh
screenshots after actions. Reuse the thread's remote runner and credentials.
Keep screenshots intact through Responses, Anthropic, Gemini and ACP replay.
Read large driver results through the runner filesystem rather than truncated
Blaxel process stdout. Include pinned driver provisioning and a live native
calculator acceptance harness.

Add an explicitly selected local macOS backend using the signed CuaDriver app's
daemon. Preserve approvals and image replay, fence cancelled native input,
invalidate grounding across local sessions and daemon generations, and keep
large accessibility trees usable inline. Qualify native macOS Calculator via
the public Roder runtime plus an owned AppKit input fixture. Local macOS never
falls back from a remote runner thread. Exact semantic value replacement has
no delivery-mode argument on the shared canonical tool surface.

Include a full XFCE/X11 desktop workflow with native GUI app launching, file
drag, ODT creation/save/reopen and independent file grading. Add a local
read-only live screen viewer and preserve plain-text Linux driver refusals
without discarding their error status or replaying input.

## 0.3.5 (2026-10-08)

### Fixes

#### Report context size after routed compaction

Complete context assembly after the selected provider compacts the first
inference request, so clients receive the size of the compacted prompt.

#### Preserve native compaction across OAuth and routed turns

Codex OAuth sessions require provider-owned compaction. Defer automatic
compaction until routing selects the provider, and keep local fallback available
for custom Responses providers that do not implement OpenAI native compaction.

#### Keep OpenAI Responses compaction provider-owned

Use OpenAI native compaction for automatic, manual, and context-limit recovery,
including models absent from the local catalog. API-key and Codex subscription
requests both use the streamed `compaction_trigger` strategy on Responses.
Preserve the full input window and encrypted boundary with retained messages;
clear parent-turn output constraints during compaction. Propagate native failures
instead of replacing conversation state with Roder text summaries.

## 0.3.4 (2026-10-07)

### Fixes

#### Align thread goals with Codex

Protect unfinished goals, support explicit model-requested pause, account in-flight
usage, enforce exhausted budgets, preserve stopped states when editing, and audit
completion and repeated blockers before ending autonomous goal work.
Automatic continuation preserves turn configuration and rechecks goal state
under turn admission to avoid racing a pause or user turn.
Conversation forks inherit the source goal snapshot after flushing usage.

#### Align autonomous goals with the selected Full Access permissions: suppress

ordinary extension approval requests in bypass mode, refresh permission guidance
across inference and compaction, and resolve permission-switch races without
widening restricted child permissions or overriding explicit policy denials.

## 0.3.3 (2026-10-04)

### Fixes

#### Recover hosted owner drain after unconfirmed lease release

Retain sealed owner confirmation state across errors, timeouts and cancellation.
Idle expired owners can prove local quiescence without restoring authority;
active execution and failed lifecycle persistence still block completion.

## 0.3.2 (2026-10-04)

### Fixes

#### Recover external-tool execution status from durable thread history after a host restart. Unresolved requests report an uncertain outcome and cannot be replayed or resolved by a replacement executor.

Persist external execution requests before delivery and resolution receipts before acknowledgement. Persistence failures prevent delivery or successful acknowledgement.

Flush JSONL event writes before append returns so immediate process exit cannot drop a delivered execution receipt.

## 0.3.1 (2026-10-03)

### Features

#### Add tenant-scoped runtime ownership leases with database-clock expiry and monotonic generations. Expired and superseded owners cannot renew or release a replacement owner. Fence session, checkpoint, event, and artifact writes within owner-locked transactions; unbound handles cannot write once ownership is enabled for a tenant. Add an optional monotonic runtime execution lease: reject turn/tool admission after loss, recheck after approval waits, and close hosted sockets instead of delivering stale-owner notifications. Add bounded host-backed renewal supervision that revokes authority and drains local work on loss or uncertainty. Already dispatched external actions still require host-level reconciliation after a crash.

Add one-hop authenticated owner forwarding and discard revoked cached runtimes so reconnects can resolve ownership again. Hosts must supply a trusted owner endpoint and shared replica authentication/policy.

Add idle owner sealing that waits for admitted turns, tool futures, and cleanup without cancellation, followed by bounded durable release. Hosts must stop new inbound work and only treat confirmed release as a handoff receipt.

Add replica pool drain admission and release polling. Preserve active-work recovery messages, reject new work, keep readiness separate from liveness, and retain failed release receipts. Count forwarded sockets so an empty local runtime pool cannot falsely report a fully drained replica.

Expose an optional, bounded same-port HTTP lifecycle handler for host-authenticated rollout commands. Update the hosted distribution to leave it disabled by default.

Drain forwarded sockets through an authenticated owner registration without consuming repeated user request-rate tokens. Preserve active browser results, release only idle owners, cancel registrations on rollback, and reconnect read-only subscriptions without retiring their owner.

Update dependent distributions and the TUI for the breaking hosted options, session-store configuration, and MySQL configuration module APIs.

### Fixes

- Preserve event history across runtime restarts and concurrent writers by allocating durable MySQL sequence numbers and deduplicating by event identity. Upgrade existing event tables without replacing stored payloads.

## 0.3.0 (2026-10-03)

### Breaking Changes

#### Bind hosted external tools to one authenticated connection

Hosted external tool threads require an executor binding before a turn starts.
Only that connection receives execution requests and may resolve them with the
current lease and turn. Disconnect, unbind, and takeover terminate pending
requests. Metadata readback supports recovery without replaying effects.
The TypeScript SDK adds a generic executor helper with cancellation and duplicate
request suppression for hosts that own their existing notification loop.

Custom TypeScript transports expose a synchronous closedSignal so executor callbacks abort immediately on transport loss. The unused local startupTimeoutMs option is removed.

Release the reverse dependency closure together so registry builds share the new protocol and core types.

## 0.2.1 (2026-10-01)

### Features

#### Native OpenAI Responses computer use

Register the native `computer` tool for an explicitly bound CDP browser, execute
ordered action batches with retained per-thread sessions and cancellation
cleanup, and return original-detail `computer_screenshot` observations through
`computer_call_output`. Preserve native call identities through transcript
replay, WebSocket continuation, and ACP tool updates. Include an independent
real-browser protocol eval and a live OpenAI eval runner.

Accept the native API's nullable mouse modifiers. Initialize TLS and large
worker stacks in the live runner, preserve call execution order in its trace,
and independently verify the final browser UI and masked screenshot.

### Fixes

#### Validate native browser registration and recover failed input cleanup

Declare the native computer tool in Chrome's extension manifest, reserve its
name from external function tools, and reconcile cancelled cached sessions.
Retry failed input releases before reusing a session and attempt all key, mouse,
and screenshot-mask cleanup independently. Scrub overlapping secrets and every
nested element observation. Reject malformed Desktop origin and tab settings,
check origin scope before resolving targets, and disable arbitrary eval under
an origin ceiling. Honor the configured eval provider endpoint, validate native
compaction, preserve generic image-reopen guidance, and accept documented empty
Jev completion conditions and sparse serialized controls.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

### Fixes

#### Align apply_patch parsing, ordered line matching, custom-tool grammar, model routing, and streamed input with Codex. Use one canonical patch argument and Codex patch syntax for local and remote tools. Fix split UTF-8 SSE decoding, terminal completion, client tool-search continuation contracts and exhaustion, replay, pooled HTTP transport, and Retry-After handling. Retry transient sampling failures across runtime profiles, stop on terminal completion, reject silent EOF, preempt sampling on steering, and emit terminal failures once.

Persist completed messages, reasoning, search exchanges and tool effects during sampling. Execute opted-in reads eagerly through normal policy and authorization checks. Add task-scoped WebSocket pooling with verified delta continuation, interruption invalidation and HTTP fallback. Add provider-native, task-aware compaction, complete opaque-window replay, request byte/image guards, failure identifiers, streamed proposed patch progress, actual filesystem diffs and uncertainty-aware partial outcomes and rollback.

Require canonical completion from mock tool responses and keep sampling steering separate from turn cancellation, including immediate-interrupt races. Keep manual compaction and new input atomic for its target task. Prevent concurrent eager reads from deadlocking streamed item persistence. Default new evaluation runs to GPT-6-luna.

## 0.1.20 (2026-09-26)

### Fixes

#### Publish today's content under fresh crate versions

`roder-api` 0.1.21, `roder-core` 0.1.19, `roder-app-server` 0.1.17,
`roder-evals` 0.1.2 and `roder-ext-openai-responses` 0.1.10 were published to
crates.io on 2026-09-23 from a branch that predates the Codex agent backend,
the Jev browser tool, the Chrome bridge work and the Grok 4.7 catalog entries.
crates.io versions are immutable, so those numbers cannot carry the current
code and the registry copies do not match the git tags of the same version.

Bump them so the released content reaches crates.io under versions that
describe it. `roder-ext-jev` and `roder-ext-codex-backend` are bumped with
them: both are new crates whose first publish must depend on a registry
`roder-api` that actually contains `roder_api::backend`, which the 0.1.21
registry copy does not.

Adds the package-local READMEs both new crates need to publish.

## 0.1.19 (2026-09-26)

### Features

#### Per-turn OpenAI service tier (Fast mode), and GPT-6 Sol and Luna

A caller can now choose the OpenAI service tier for a turn.
`StartTurnRequest::service_tier_override` (for example `"priority"` for Fast
mode) is carried to every inference round of the turn as
`RuntimeHints::service_tier`. The OpenAI Responses provider sends it as the
top-level `service_tier` request field only on the OpenAI profile; OpenRouter,
xAI, and Fireworks never receive it. `None` keeps the provider default.

The tier OpenAI reports it actually served (`response.service_tier`) is recorded
on `TokenUsage::service_tier`, so a biller can tell a request that ran fast from
one that was downgraded to `"default"` under load. Both new fields are optional
and default to absent when older payloads are deserialized.

`StartTurnRequest` gains a public field, so code that builds it with a struct
literal must add `service_tier_override: None`.

The OpenAI/Codex catalog adds `gpt-6-sol` (efforts `low` through `max`, like
`gpt-6-astra`) and `gpt-6-luna` (efforts `low` through `max`, like
`gpt-5.6-luna`), so per-turn reasoning validation accepts them.

## 0.1.18 (2026-09-12)

### Features

- Add Codex-compatible local project hooks with lifecycle diagnostics and expandable TUI output.

### Fixes

#### Fix local hook denials, chaining, and session scope

- A `PreToolUse` deny with a blank or missing reason no longer falls open. The
  denial is the decision, not the prose; a bare `permissionDecision: "deny"`
  (and the legacy top-level `decision`) now blocks the call and supplies a
  stand-in reason for the transcript.
- Chained `PreToolUse` hooks each see the input as the previous hook left it.
  The payload was built once from the model's original arguments, so a second
  rewriting hook was handed stale input.
- `SessionStart` fires once per session instead of once per turn. It now hangs
  off `ThreadCreated` (`startup`) and `ThreadLoaded` (`resume`) rather than
  `TurnStarted`, which re-ran setup hooks on every user message.
- A tool call with no matching hook keeps the model's original `raw_arguments`
  string. Every call was being re-serialized from parsed JSON whether or not a
  hook ran.

#### Fire the `SessionStart` local hook once per session

`SessionStart` hung off `ThreadLoaded`, which is emitted on every read of a
thread from its store, so a single `roder exec` run executed its setup hooks
four times — including once after the turn had already stopped. The dispatch is
now claimed once per thread, whichever event opens the session.

#### Wake `wait_agent` on mailbox activity queued before it starts waiting

`wait_agent` checked `has_pending_turn_steers`, which only sees activity that
delivery already converted into a turn steer. A message queued while the
recipient's turn was not yet registered stayed in the mailbox undelivered, so
the waiter slept through it and returned only on timeout. It now also consults
the mailbox directly, and the lifecycle test removed for flaking is restored —
it passes 20 runs in a row.

#### Remove the flaky `wait_agent_observes_mailbox_activity_queued_before_subscription`

lifecycle test. It failed roughly one run in four because the wakeup it asserts
is genuinely racy, not because the test was written wrong — see the changeset
body for the underlying gap.

#### Tighten local hook output, matchers, and inert handlers

- `additionalContextLimit` is honoured instead of parsed and dropped, and output
  is trimmed to a byte budget on a char boundary. The cap counted chars, so
  multi-byte output reached four times its intended size.
- An unparseable `matcher` reports itself as a failed hook run rather than
  silently disabling its hook.
- `prompt` and `agent` handlers record as `skipped`, not `success`, and can no
  longer be read as a permission decision.
- `hooks.json` is parsed once per file version rather than on every dispatch,
  keyed on modified time and length.

## 0.1.17 (2026-08-06)

### Fixes

#### Fix DeepSeek thinking mode reasoning in Ctrl+P and tool rollouts

DeepSeek models advertise real thinking efforts again, stream
`reasoning_content`, send the DeepSeek `thinking` toggle, and pass CoT back on
tool-call turns.

## 0.1.16 (2026-08-05)

### Features

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

## 0.1.15 (2026-08-04)

### Fixes

#### Recover from Grok prompt-length overflows and advertise xhigh

Detect xAI/Grok `maximum prompt length` (and related context-overflow) errors as
context-limit failures, then force-compact and retry the live turn in place
(with a second attempt that strips the last bulky item). Compaction summary
inference shrinks its head and falls back to deterministic summaries when the
summary request itself overflows. Grok reasoning catalogs now include `xhigh`.

## 0.1.14 (2026-08-03)

### Features

#### Add `/review`: a read-only review sub-turn with structured findings and pluggable review publishers

`/review` runs a detached, read-only reviewer over the working diff, a base
branch, a commit, or a free-form scope, and returns prioritized findings with
file/line locations. Findings render in a new TUI panel where they can be kept
or dropped, and are exposed over the app-server as `review/start`,
`review/publish`, and `review/publishers/list` plus `review/started`,
`review/completed`, `review/failed`, and `review/published` notifications.

Publishing goes through a new `ReviewPublisher` extension service. The
first-party `roder-ext-github-review` publisher submits findings as GitHub pull
request review comments over the `gh` CLI or the REST API, with diff-hunk
prefiltering and a dry-run mode. Configure it under `[review]` and
`[review.publishers.github]`. Each `[review.publishers.<id>]` block is stored
opaquely and parsed by the publisher's own crate, so adding a platform does not
change the core config types.

Also fixes `roder app-server`, which built its Tokio runtime in current-thread
mode. Providers that bridge a synchronous callback back into async work call
`tokio::task::block_in_place`, which panics outright on a current-thread
runtime, so the `claude-code` provider aborted the server on its first
Roder-executed tool call. The app-server now uses a multi-threaded runtime like
the TUI entry point.

## 0.1.13 (2026-07-23)

### Features

#### Parallel search + extract web tools

Fix Parallel.ai Search against the current V1 API (`advanced_settings` for
max_results/domain filters), add `parallel_extract` for URL markdown extraction,
auto-install Parallel tools when it is the selected web_search provider, and
inject short Parallel web-access instructions into the developer prompt when
those tools are available.

## 0.1.12 (2026-07-21)

### Fixes

#### Honor explicit turn deadlines in interactive runtimes

Apply a configured wall-clock turn deadline to interactive profiles while preserving the unbounded default when no deadline is configured.

## 0.1.11 (2026-07-21)

### Fixes

#### Add provider-authoritative remote workspace execution leases

Allow remote runner providers to fence a complete multi-step workspace tool
execution across runtimes or replicas. Roder now bounds lease acquisition,
stops execution if the provider loses the fence, releases it on every normal
tool outcome, and refreshes command deadlines after waiting for the fence.

#### Serialize shared remote runner tools and refresh their deadlines

Prevent concurrent agent threads attached to the same remote runner session from interleaving multi-step workspace tool operations. Recompute the tool deadline after lazy runner provisioning and the shared-session queue so command leases use the time that actually remains.

## 0.1.10 (2026-07-21)

### Features

#### Lazily initialize per-thread remote runners and support one-shot exec

Runner-bound threads now create or resume their remote session only when an
approved native workspace tool first executes. Concurrent first tool calls
share one initialization, the live session is reused across later tools and
turns, and its state is persisted before the first tool runs so a new process
can rejoin it. Text-only and host-executed MCP turns do not wake a runner, and
failed initialization remains retryable without falling back to local tools.

`exec_command` now runs non-interactive one-shot commands through a remote
runner with remote working-directory scoping, shell/login handling, deadlines,
timeouts, output truncation, and the existing Codex-shaped result payload.
Remote TTY and stdin-continuation requests fail clearly instead of executing
on the host. Hosted runtimes can also disable local workspaces completely, so
a missing or malformed runner binding fails closed before any native workspace
tool can touch the host filesystem.

### Fixes

#### Read lifecycle state without loading full threads

Thread stores can now load persisted extension state directly. Lifecycle-only
reads use that seam, so metadata-only thread reads do not need to project a
full event, turn, and item snapshot.

## 0.1.9 (2026-07-21)

### Features

#### Eval loop persistence: continue past tool-failure limit and nudge on empty finalization

Adds two `[reliability]` knobs so eval-style runs keep working instead of
finalizing early:

- `continue_on_failure_limit` (default `false`): when a turn hits
  `max_consecutive_tool_failures`, reset the consecutive-failure counter, nudge
  the model to keep going, and continue the round loop instead of stopping.
  Bounded by `max_tool_failures_per_turn`, `max_model_calls_per_turn`, and the
  per-turn tool-round cap.
- `empty_tool_call_nudges` (default `0`): number of times a non-interactive/eval
  turn is nudged to verify completeness when the model returns a final message
  with no tool calls, before genuinely ending the turn.

Both default to the safe (off) behavior for interactive and plain
non-interactive runs; the `eval` runtime profile enables them by default
(`continue_on_failure_limit = true`, `empty_tool_call_nudges = 1`), and explicit
`[reliability]` config keys still override the profile defaults. Also strengthens
the eval-profile persistence instructions to keep working until the task is fully
solved and verified.

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

### Fixes

#### Fix provider compaction thrashing and show token/duration summaries

Persist OpenAI/Codex compaction items as soon as the stream emits them so a
later SSE decode failure cannot drop the boundary and re-compact every round.
Surface before/after estimated tokens and elapsed time in the TUI and
app-server item stream.

#### Honor provider compaction boundaries

Drop pre-compaction history after OpenAI server-side compaction items so long sessions no longer re-send and re-compact the full window every request. Treat provider compaction as a local transcript boundary, and map Anthropic local context summaries correctly when the emergency client path runs.

#### Clarify subagent role selection and native full-history labels

Model-facing subagent tools now advertise configured roles, reject lane names
used as roles before fanout, and report lane/tool incompatibilities before a
child agent runs. Native `spawn_agent` full-history forks now accept their
advisory `agent_type` label while continuing to reject model, provider, and
reasoning overrides.

## 0.1.8 (2026-07-10)

### Features

#### Match Codex V2 Ultra agent lifecycle semantics

#### Added

- Added Codex V2-style canonical agent trees, full/empty/last-N context forks,
  nested agents, reusable follow-up turns, mailbox-aware waiting, and
  non-destructive interruption.
- Added exact parent model, provider, Ultra reasoning, workspace, policy, tool,
  runner, and live developer-context inheritance for spawned agents.
- Added full `team/started`, `team/member/started`, and terminal result details
  to the app-server protocol.

#### Changed

- `send_message` now queues coordination without starting an idle agent, while
  `followup_task` starts or steers the existing canonical agent thread.
- Child final results and terminal errors are delivered automatically to their
  direct parent, and completed identities remain available for later work.
- Inter-agent delivery now uses typed `MESSAGE`, `NEW_TASK`, and `FINAL_ANSWER`
  envelopes with canonical sender and recipient paths.

#### Fixed

- Fixed spawn-capacity, completion/follow-up, interruption, mailbox batching,
  acknowledgement, restart, and wait races found by comparison with Codex V2
  and Claude Code agent workflows.
- Prevented full-history children from replaying parent orchestration by making
  the newest `NEW_TASK` payload the authoritative child assignment.
- Preserved spawn-time live instructions, developer context, and model
  selection across reusable follow-up turns.
- Prevented interrupted-turn mailbox reservations from stranding queued
  messages or accepting stale delivery acknowledgements.
- Bounded recursive agent paths to five levels below `/root`, rejecting deeper
  spawns before creating team or thread state.

## 0.1.7 (2026-07-09)

### Features

#### Add GPT-5.6 Codex models and Ultra mode

Expose GPT-5.6 Sol, Terra, and Luna plus GPT-5.4 in the OpenAI and Codex
catalogs, with the current context windows, defaults, and reasoning-effort
menus. Make Sol the default Codex model.

Keep Ultra as a first-class Roder effort for Sol and Terra while mapping it to
the provider's `max` wire effort. Ultra enables proactive, bounded multi-agent
delegation; lower Sol and Terra efforts remain explicit-request-only.

## 0.1.6 (2026-06-30)

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

## 0.1.5 (2026-06-26)

### Features

#### Live agent_swarm progress events

Emit an `AgentSwarmProgress` `RoderEvent` each time a swarm child resolves,
carrying a running `completed/failed/aborted/total` snapshot (roadmap 104,
Task 1 follow-up). This lets a client render a live "N/total done" tick between
`AgentSwarmStarted` and `AgentSwarmCompleted` instead of only the final result.
The scheduler reports incremental progress through a new
`AgentSwarmProgressObserver`; the `agent_swarm` tool bridges it onto a runtime
`AgentSwarmProgressSink` supplied on the tool-execution context, which the
runtime backs with the event bus (and thread-event persistence). Children do
not publish progress; only the lead swarm does.

#### Per-thread agent-swarm mode

Scope agent-swarm mode to a single thread instead of the whole runtime (roadmap
104 follow-up), so toggling swarm mode on one thread no longer leaks the swarm
reminder into other threads sharing the runtime — aligning swarm mode with the
per-thread `thread/*` contract.

The runtime keeps a per-thread override map alongside the global default
(mirroring the team per-member policy-mode idiom): `set_agent_swarm_mode_for_thread`
stores the override and emits `AgentSwarmModeChanged` with the real `thread_id`,
and `effective_agent_swarm_mode_for_thread` resolves the per-thread override or
falls back to the runtime-global default; the turn loop now consults it. The
`thread/set_agent_swarm_mode` app-server method gains an optional `threadId`
(present -> per-thread, absent -> global, preserving legacy behavior) echoed back
in the result, and the TUI passes its current thread id. `settings/get` continues
to expose the global default. Covered by runtime unit tests (per-thread
isolation, off-override-wins, real-thread-id event) and an app-server e2e test.

## 0.1.4 (2026-06-26)

### Features

#### Agent-swarm lifecycle events on the event bus

Emit `AgentSwarmStarted` (with the child count) and `AgentSwarmCompleted` (with
completed/failed/aborted counts) `RoderEvent`s when the `agent_swarm` tool runs,
so any app-server/SDK/TUI client can observe a swarm as a whole rather than only
the per-child `Subagent*` traces (roadmap 104, Task 1). The runtime emits these
around tool routing; existing notification mappers fall through their catch-all
arms, so no client breaks.

#### Server-side agent-swarm mode

Move agent-swarm mode from TUI-only client state to runtime/app-server state so
every client benefits (roadmap 104). Adds the `thread/set_agent_swarm_mode`
app-server method, an `agentSwarmMode` field on `settings/get`, and an
`AgentSwarmModeChanged` event. When swarm mode is active the runtime injects the
canonical swarm reminder into each turn's developer instructions
(`Runtime::set_agent_swarm_mode` + `apply_agent_swarm_mode`), so the model is
nudged toward the `agent_swarm` fanout tool regardless of which client drove the
turn. The TUI now toggles swarm mode through the method and no longer prepends
the reminder client-side. Also fixes two pre-existing method-manifest ordering
issues (`auth/kimi-code/*`, `thread/compact`).

### Fixes

#### Enforce agent_swarm as the only tool call in a response

The core turn loop now denies any model response that mixes `agent_swarm` with
other tool calls, or issues multiple `agent_swarm` calls at once (roadmap 104,
Task 2). Each call in the offending batch gets an error tool result with
actionable retry text, so the model re-issues `agent_swarm` by itself and every
`tool_call_id` still receives a response (keeping chat-completions transcripts
valid). Adds `roder_api::subagents::agent_swarm_batch_violation` and the shared
`AGENT_SWARM_TOOL_NAME` constant.

## 0.1.3 (2026-06-22)

### Fixes

- Stabilize Roder startup, streaming responses, and provider behavior

#### Interleave in-stream reasoning/text with tool calls in the transcript

Providers like `claude-code` run their entire tool loop inside a single
`stream_turn`, streaming many assistant messages, thinking blocks, and tool
calls before the turn completes. The turn loop previously coalesced every
reasoning/text chunk into one trailing `ReasoningSummary` + `AssistantMessage`
that was persisted only after all the in-stream tool calls (which the runtime
tool executor persists in real time). The result was a transcript where every
tool row appeared first and all the per-step thinking/narration collapsed into a
single block at the end, so the activity between tool calls looked missing.

The runtime now keeps a shared per-turn buffer of the reasoning + final-answer
text streamed so far. `RuntimeTurnToolExecutor::execute` flushes that buffer as
discrete `ReasoningSummary` + `AssistantMessage` items immediately before it
persists each in-stream tool call, and the turn-end persistence writes only the
remaining (post-last-tool) content. The persisted order is now
`reasoning -> text -> tool -> reasoning -> text -> tool -> ... -> final answer`.
This is gated to providers that execute tools in-stream (currently
`claude-code`); all other providers are unchanged.

#### Increase the runtime event-bus capacity so heavy turns stop dropping rows

The runtime broadcasts every event (including a per-delta `InferenceEventReceived`
firehose) through a single broadcast ring buffer. The TUI drains it on its render
loop, which during an active turn only wakes at the ~6 FPS status-animation
cadence — backend events do not wake the input poll, so up to ~166ms of events
buffer between drains. A reasoning-high provider that runs its whole tool loop in
one turn (e.g. `claude-code`) bursts far more than 1024 events into that window,
overflowing the ring and silently dropping tool/thinking rows from the live view.

The bus capacity is raised from 1024 to 16384 (`EVENT_BUS_CAPACITY`), giving ~16x
headroom across streaming bursts and brief render stalls. The existing `Lagged`
handling and stuck-turn watchdog still cover any pathological overflow, so the UI
can never hang even if the buffer is exhausted.

#### Kimi Code OAuth chat requests omit unsupported OpenAI-compat fields

OAuth turns on `api.kimi.com/coding/v1` no longer send `stream_options` or
`parallel_tool_calls`, which caused 400 responses on the managed Kimi Code API.
Adds configurable flags on the shared chat-completions helper and gates
`should_compact_transcript` to test builds only.

## 0.1.2 (2026-06-16)

### Fixes

- Improve context compaction across phases 2–4: prune old tool outputs before full compaction, add LLM state-snapshot summarization with verify/reject, hysteresis coalescing, `/compact` via `thread/compact`, `context.compaction_skipped` metrics, and a Grok-style loop regression fixture. Phase 1 fixes remain: compaction boundary on load, once-per-turn guard, ProviderMetadata exclusion from token estimates, and suffix retention from the last user message.

## 0.1.1 (2026-06-15)

### Features

#### First-party image generation providers (OpenAI GPT Image and Google Gemini Nano Banana)

Provider-neutral image generation through the core media API: an image-capable
`MediaGenerationRequest`/multi-output `MediaGenerationResponse` contract, a new
`ProvidedService::MediaGenerator` extension service, a runtime media generation
service backing the canonical `media_generate_image` tool with a deterministic
offline fallback, new `roder-ext-openai-images` (`gpt-image-2` plus legacy ids)
and `roder-ext-google-images` (Nano Banana 2/Pro/base) provider crates,
`[media.image_generation]` config, `media/image/providers/list` and
`media/image/generate` app-server methods, `roder media` CLI commands, palette
entries, and regenerated schemas/SDK stubs. Live provider smokes stay opt-in
behind `RODER_OPENAI_IMAGE_LIVE` / `RODER_GEMINI_IMAGE_LIVE`.

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
