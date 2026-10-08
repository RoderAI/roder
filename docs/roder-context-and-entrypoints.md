# Roder Context And Entrypoint Metrics

Roder treats context as a measured runtime resource. Context assembly emits trace metadata for block sizes, total estimated context tokens, and entrypoint hints before the model sees the turn.

## Runtime Signals

- `context.block_added` records the block type, byte count, estimated tokens, and priority for provider-supplied context blocks.
- `context.assembly_completed` records total context block count, total bytes, estimated tokens, and the active token budget when one is available.
- `context.entrypoint_candidates_injected` records how many bounded entrypoint candidates were injected and how large the injected hint block is.
- `context.compaction_recorded` records original versus compacted item counts and estimated tokens, including whether file-backed context artifacts were used.
- `tool.output_truncated` records original tool output line/character counts, inline character count, and whether the output was moved to a context artifact.

## OpenAI Responses Compaction

OpenAI Responses models use provider-owned compaction for automatic watermarks,
manual `/compact`, and context-limit recovery. API-key and Codex subscription
requests both use OpenAI's streamed `compaction_trigger` strategy on
`POST /responses` (or the Responses WebSocket transport), matching Codex CLI.
The trigger is the final input item. Roder forwards the full conversation window
without its local tool-output pruning, preserves instructions and tools, and
clears parent-turn output constraints. It persists the returned encrypted
compaction boundary alongside retained client messages for the next request. Native errors propagate instead of
falling back to Roder's LLM or deterministic text summaries. Model support is
decided by OpenAI, including models absent from Roder's catalog.

The runtime triggers explicit compaction at the configured watermark before
the context window fills. Normal Responses requests also support OpenAI's
server-side `context_management` compaction. These rules apply equally to
app-server and ACP sessions; their event and capability contracts are unchanged.

See [OpenAI's explicit trigger documentation](https://developers.openai.com/api/docs/guides/reasoning)
and [compaction guide](https://developers.openai.com/api/docs/guides/compaction).

## Entrypoint Planner

The default extension host registers `EntrypointContextPlanner`. It uses fresh filesystem traversal, recent git changes, and bounded scan-mode search from `roder-search`. It does not rely on a stale index as the source of truth, and it does not inject full file contents. The model receives a compact `EntrypointHint` block with likely file paths and short reasons.

## Eval Metrics

Context eval reports include:

- `context_estimated_tokens`
- `context_bytes`
- `entrypoint_candidates`
- `entrypoint_injection_event`
- `first_relevant_file_read_event`
- `irrelevant_file_reads`
- `truncation_follow_ups`
- `tool_output_truncations`

Run focused context checks with:

```sh
cargo test -p roder-core context
cargo test -p roder-evals context
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals cargo run -p roder -- eval run evals/fixtures/context --offline
```
