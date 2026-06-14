# Roder Context And Entrypoint Metrics

Roder treats context as a measured runtime resource. Context assembly emits trace metadata for block sizes, total estimated context tokens, and entrypoint hints before the model sees the turn.

## Runtime Signals

- `context.block_added` records the block type, byte count, estimated tokens, and priority for provider-supplied context blocks.
- `context.assembly_completed` records total context block count, total bytes, estimated tokens, and the active token budget when one is available.
- `context.entrypoint_candidates_injected` records how many bounded entrypoint candidates were injected and how large the injected hint block is.
- `context.compaction_recorded` records original versus compacted item counts and estimated tokens, including whether file-backed context artifacts were used.
- `tool.output_truncated` records original tool output line/character counts, inline character count, and whether the output was moved to a context artifact.

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
