# OpenAI compaction audit, 2026-10-08

Reference: local Codex checkout, commit
`87be737b664508d48402f08f57d09aba48e5f17a`. No upstream fetch was performed;
this report compares the requested local reference, not the latest remote Codex.
Roder was inspected in the task checkout, including the
native-only fixes made during this audit. No live OpenAI requests were run.

## Conclusion

Roder initially implemented OpenAI's standalone `/responses/compact` endpoint
for API keys and remote V2 (`compaction_trigger` on `/responses`) for Codex
subscriptions. It did
not reliably keep compaction provider-owned. The fixes below remove the paths
that could replace OpenAI history with Roder summaries. Roder is still not at
full parity with the reference's history budgeting and model-switch handling.

The reference now selects remote V2 for OpenAI and Azure Responses providers
(`codex-rs/model-provider/src/capabilities.rs:29`). It uses a normal streamed
Responses request with current instructions, tools, and a compaction trigger
(`core/src/compact_remote_v2_attempt.rs`). Following the user's clarification,
Roder now uses that same streamed V2
request for API-key and Codex subscription compaction. The standalone endpoint
branch has been removed. OpenAI documents explicit `compaction_trigger` input on
`/responses`: https://developers.openai.com/api/docs/guides/reasoning.
Normal inference can still use server-side `context_management` as documented at
https://developers.openai.com/api/docs/guides/compaction.
Existing persisted compacted windows continue to replay unchanged.

## Defects fixed in this working tree

1. **P1: Native overflow fell back to Roder summarization.**
   `roder-core/src/provider_compaction.rs` converted native context-window and
   request-size failures to `Unsupported`; `transcript.rs` then built an LLM or
   deterministic summary. OpenAI Responses now declares native compaction
   mandatory; these failures propagate. Other providers retain their existing
   behavior. Codex selects its local summarizer only for providers without
   remote support (`core/src/session/turn.rs:1472`).
2. **P1: Pre-turn compaction used the runtime default provider.**
   The runtime resolved the chat's provider, but `transcript_for_turn` independently
   selected the global default unless a per-turn override was present. A chat
   using OpenAI could be compacted by another engine before inference. The
   resolved provider now travels with the model into pre-turn compaction.
3. **P2: Catalog misses disabled native compaction.**
   `roder-ext-openai-responses/src/native_compaction.rs` required a local model
   entry advertising compaction. New model IDs and custom aliases could fall
   through to text summaries. The OpenAI endpoint now decides model support.
4. **P2: Explicit API compaction could be delayed until overflow.**
   The native-capable model branch of the local trigger waited for the entire
   context window. Required-native engines now use the normal watermark, as
   Codex subscription compaction already did. This preserves room for the
   standalone compact request to fit.
5. **P2: Generic tool pruning preceded native compaction.**
   Old tool evidence could be replaced with Roder's prune notice before OpenAI
   saw it, or pruning could suppress compaction. Required-native engines bypass
   that generic pruning. The reference instead has a narrowly scoped trimming
   pass that runs only while the compaction request exceeds the context window
   (`core/src/compact_remote_history.rs:76`).

## Remaining findings

### P1: Streamed retained-message budget is not actually bounded

Roder `roder-ext-openai-responses/src/native_compaction.rs:77` retains recent
user/developer/system messages using 40,000 serialized bytes. Its first retained
message is accepted regardless of size; later messages that do not fit are
skipped, so older small messages can backfill around a missing recent message.
A very large latest user message or image can leave the post-compaction request
too large, resulting in repeated native failures. This retention code now applies to both API-key and subscription compaction.

The reference budgets 64,000 tokens, truncates the boundary message to available
budget, tracks attached image notices, and optionally budgets images
(`core/src/compact_remote_v2.rs:73,494,610` and `compact_remote_v2_images.rs`).
It retains user/hook messages and explicitly identified client-authored developer
messages; Roder has no equivalent provenance filter for developer/system messages.
A follow-up should implement token-bounded V2 retention with recent-message
continuity and image handling, while preserving already persisted windows.

### P1: No compaction compatibility check on model switches

Roder `roder-core/src/runtime.rs:5977` appends a model/tool-profile notice and
compacts using the selected current model. It has no compaction compatibility
hash and no previous-model compaction step. A switch to a smaller context window
can reject the old window or its encrypted state before new compaction can run.

The reference compares `comp_hash`, compacts with the previous model when
required, and has a bounded fallback to the current model
(`core/src/session/turn.rs:1340` and `compact_model_fallback.rs`). This needs model
metadata and explicit tests for downshifts and incompatible opaque boundaries.

### P2: Emergency fit recovery is weaker than the reference

After these fixes, Roder correctly reports an oversized native request, but it
has no equivalent of Codex's fit-only trailing tool-output rewrite. Generic
pruning is unsuitable because it can discard evidence even when the full
compaction request fits. Add a provider-window fit check and minimal trailing
output rewrite, preserving call IDs and pairs, before retrying native compaction.

### P2: Trigger accounting and settings can disagree

Roder `roder-core/src/transcript_compaction.rs` uses the runtime-global threshold
or catalog threshold and estimates transcript characters plus opaque/image
state (`compaction.rs:59`, `prompt_accounting.rs`). The normal inference request
also consults model-profile threshold overrides (`runtime.rs:5725`). Explicit
compaction does not use those overrides or include instructions/tool schemas
in its threshold estimate. The reference's context-window status uses observed
active context and prefix/body scopes (`core/src/session/context_window.rs`).
Large tool schemas or instruction bundles can therefore make the actual request
fill before Roder's local trigger. Unify effective threshold resolution and
include the outgoing prompt overhead in estimates; keep estimates distinct
from provider usage and encrypted-state byte size.

### P2: Cached compaction instructions and tools can become stale

`roder-core/src/runtime/compaction_template.rs:81` returns a cached request after
refreshing goal instructions, while `provider_compaction.rs` updates its model
and transcript. Model-specific tools and ordinary instructions can still reflect
a prior sampling request. Codex builds a compaction prompt from the captured
current step's tool router and base instructions
(`core/src/compact_remote_v2_attempt.rs:103`). Rebuild or invalidate the template
when model, tools, permissions, or instruction context changes.

## Intentional differences and matching behavior

- Streamed compaction uses `/responses`, retains current instructions/tools,
  clears structured output and forced tool choice, and appends one final
  `compaction_trigger`. Offline HTTP coverage checks both authentication profiles,
  uncatalogued models, streamed usage/completion, persistence, and next-turn replay.
- Persisted `compacted_input` windows replay without pruning retained messages
  before their boundary (`response_replay.rs:139`).
- Roder commits completed opaque boundaries before terminal stream completion
  so a disconnect does not lose a boundary. Codex installs replacement history
  after `response.completed` and validates exactly one compaction item
  (`compact_remote_v2.rs:430`). Roder deliberately retains its earlier durability
  policy; its retry regression checks reuse the committed window.
- Roder supports steering preemption and retry budgets, but uses its configured
  general provider budget; Codex caps V2 stream retries at two. This is a policy
  difference, not an authentication restriction on streamed compaction.
- Roder manual compaction waits for idle thread admission. Public app-server
  `thread/compact` routes through the same runtime. This change adds no ACP wire
  methods or advertised capabilities.

## Validation

- Original native-only validation: 241 API, 266 core, and 123 Responses provider
  tests passed. After enabling the API-key streamed trigger, all 123 Responses
  provider tests pass again; the HTTP fixture checks API-key and Codex profiles
  with both catalogued and uncatalogued models. All four core native-compaction
  regressions also pass on rerun.
- Added regressions cover watermark-triggered native compaction, preservation
  of old tool outputs, native error/unsupported handling, and pre-turn provider
  selection.
- Public JSON-RPC regression passes for native success and native context-limit
  failure through `thread/compact`, including persisted opaque state and absence
  of Roder summary records.
- `cargo test --workspace` stopped at the unrelated process-host test
  `subagent_dispatch_failure_and_timeout_are_explicit` after 2,495 passing tests.
  Its assertion at `crates/roder-ext-process-host/tests/host.rs:361` found no child
  cancellation event. An isolated rerun passed; the full workspace run remains
  incomplete rather than certified green. No process-host code was changed.
- Formatting, whitespace, and generated knope configuration checks pass.
- Remaining findings are source-audit conclusions; no live model-switch, image
  retention, or overflow reproductions were run.
