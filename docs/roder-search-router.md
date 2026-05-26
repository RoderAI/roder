# Roder Search Router

Roder's retrieval router adds compact turn-local guidance before model inference. The router does not force a tool call; it records what it recommended, what the model chose, and whether the resulting context was useful.

## App-Server Methods

All methods accept:

```json
{
  "threadId": "thread-id",
  "turnId": "turn-id",
  "limit": 20
}
```

- `retrieval/recommendations` returns bounded `RetrievalRoutePlan` records and a short debugging summary.
- `retrieval/metrics` returns measured outcomes, route accepted/ignored/failed counts, outcome counts, mode counts, and a short debugging summary.
- `retrieval/promoted` returns discovery promotion, warm-cache, expiration, and skipped-promotion state for the turn.

The methods read persisted thread events. If a thread has no thread store, or the turn predates retrieval event emission, the result can be empty even when the model searched manually.

## Reading Router Misses

A router miss means the model chose a retrieval path that did not match the route hint. Start with `retrieval/metrics`:

- `ignoredCount > 0`: the model selected a different tool family than the route recommended.
- `failedCount > 0`: Roder attempted or validated the recommended path, but the route failed before producing useful context.
- `wrong_tool_family`: the model searched in a different retrieval family, such as web search for local workspace context.
- `missing_index`: indexed search was recommended but no usable index was available.
- `stale_index`: indexed search returned evidence that may not match the current workspace.

Fallback paths are expected. A stale or missing semantic index should fall back to exact text, filename, artifact, or discovery retrieval. Treat a fallback as healthy when the final `useful` outcome has low latency and reasonable returned-token count.

## Reading Discovery State

Use `retrieval/promoted` when a turn involves tools, skills, MCP servers, plugins, or other hidden capabilities.

- `promoted`: the capability was expanded for the turn.
- `reused`: the capability was already promoted for the same thread and reused.
- `warmcachehit`: Roder had enough warm cached state to avoid another full promotion.
- `skipped`: the route intentionally did not promote the capability. The `reason` explains why.
- `expired`: prior promotion state was no longer valid and should be refreshed.

Discovery misses usually show up as `missing_promotion`, `unknown_tool`, or `wrong_tool_family` outcomes in `retrieval/metrics`. If a capability was skipped and the model then calls its full tool name or schema-dependent flow, inspect `retrieval/promoted` first; skipped promotion is often the direct cause.

## Debugging Checklist

1. Read `retrieval/recommendations` to see what the router suggested.
2. Read `retrieval/metrics` to compare the recommendation with the actual model tool choice.
3. Read `retrieval/promoted` when the recommendation involved discovery or capability promotion.
4. If semantic search missed, check index status and prefer exact grep/file fallback evidence before changing prompts.
5. If discovery missed, confirm the catalog contains the item and that promotion was not skipped because a warm-cache hit or reused promotion already satisfied the turn.
