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
