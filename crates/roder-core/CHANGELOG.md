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
