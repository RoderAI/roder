# Responses continuation duplicates: trajectory investigation

## Summary

A concrete provider-history bug can explain the model reporting duplicate
messages and tool calls even when Roder executed each call only once.

The Responses transport receives completed output items individually, but the
sampled terminal responses have an empty `output` array. The stream parser
previously remembered only that terminal array. Its WebSocket continuation then
treated the prior response as having no output and resent completed output
alongside `previous_response_id`, which already refers to that output.

This issue is present in the v0.4.2 source. The correction retains completed
streamed output instead. The terminal-only parser assignment dates to commit `19dc45e8`
(September 26, 2026); it was not introduced by the discovery startup change.

## Trajectory evidence

The investigation inspected local `events.jsonl` records, comparing canonical
`TranscriptItemAppended` items, `InferenceEventReceived` provider metadata,
and actual `ToolCallStarted` / `ToolCallCompleted` events. No private reasoning
text, prompts, tool arguments, tokens, or encrypted payloads are reproduced here.

Snapshot taken on 2026-10-28; active histories can subsequently grow:

| Thread ID prefix | Completed responses with streamed output but empty terminal output | Incremental WebSocket responses |
| --- | ---: | ---: |
| `6a879afe` | 214 | 109 |
| `7e8f9098` | 327 | 89 |
| `7da0bc8c` | 31 | 27 |
| Total | 572 | 225 |

For the first thread, provider completion event sequences 118, 193, and 416
illustrate this shape: one or more completed items were streamed, while the
terminal provider metadata's output array was empty.

Within each actual execution event kind, no `tool_id` was repeated in these
three sampled histories. Canonical transcript samples likewise did not contain
exact duplicate assistant messages or tool-call records. Repeated arguments in
legitimate polling calls are not evidence of duplicate execution.

The saved provider metadata is not a complete outgoing WebSocket wire capture.
The regression test below reconstructs the observed response shape and verifies
the actual next request emitted by the client. It does not prove that every
model comment about duplication has this cause.

## Failure mechanism

Use synthetic identifiers:

1. Client sends input `[user]`.
2. Server streams a completed reasoning item and a completed tool call.
3. Server finishes response `response_history` with `output: []`.
4. Roder's transcript correctly retains the reasoning item and tool call.
5. The transport previously saved `LastResponse.output = []`.
6. Next full client history is `[user, reasoning, call, result]`.
7. Incremental request construction strips only `[user]`, then sends:

   ```json
   {
     "previous_response_id": "response_history",
     "input": ["reasoning", "call", "result"]
   }
   ```

   These strings stand for item objects, not literal wire values.

The provider already holds the reasoning and call through the previous response.
Only the new result should be appended. Assistant messages are affected by the
same forgotten-output problem.

Relevant code:

- `crates/roder-ext-openai-responses/src/response_events.rs`:
  completed-item retention and terminal response handling.
- `crates/roder-ext-openai-responses/src/response_websocket.rs`:
  `LastResponse` and the exact-prefix check in `incremental_request`.

## Fix

- Retain each uniquely completed streamed item in the response output history.
- Preserve that history when terminal output is absent or empty.
- Use a populated terminal output list as the authoritative final list.
- Emit completion metadata with the retained output, keeping replay consumers
  consistent with the WebSocket cursor.
- Keep the strict prefix check: when a canonical assistant message differs from
  its native representation, send full history without `previous_response_id`
  rather than append a duplicate message.

The fix does not deduplicate calls by tool name or argument text. Identical
polling commands and intentional repeated operations remain valid.

## Deterministic regression

`streamed_response_history_tests.rs` uses synthetic output and a localhost
WebSocket server. Before the fix, the continuation contains three input items
(old reasoning, old call, new result); after the fix it contains only the new
result. No model request or live provider credentials are required.

```sh
mise exec -- cargo test -p roder-ext-openai-responses --lib streamed_response_history_tests
```

Additional checks cover missing terminal output, duplicate completed-item frames,
populated terminal output, and safe full-history fallback for changed assistant
message representations.

## Validation

- Before the fix, the streamed-history retention and actual WebSocket request
  regressions both failed; the latter captured three input items instead of one.
- After the fix, all four new regressions pass.
- All 167 `roder-ext-openai-responses` library tests pass.
- All 320 `roder-core` library tests pass.
- All 116 CLI binary tests pass.
- Formatting, whitespace, generated release config, and pending changeset
  structure/coverage checks pass.
- No saved history was rewritten.

## Live release validation

After the initial offline investigation, the existing opt-in evaluator was run
against the patched source with GPT-6.1 Sol, low reasoning, isolated temporary
workspaces, and the documented read-only Codex CLI credential source.

| Transport | Fixture | Result | Incremental WebSocket continuations |
| --- | --- | --- | ---: |
| WebSocket | Unicode-preserving update | Passed | 3 |
| WebSocket | Move/add/delete | Passed | 3 |
| WebSocket | Native compaction and follow-up | Passed | 3 |
| HTTP | Unicode-preserving update | Passed | n/a |
| HTTP | Move/add/delete | Passed | n/a |

All five fixtures had successful file assertions and no patch errors. The
compaction case persisted an opaque boundary and answered `BLUE` in the next
turn. The small fixture's estimated context size increased after compaction;
this verifies continuity, not token savings.

Sanitized result summary:
`docs/audits/responses-loop-eval/results/2026-10-28-streamed-history-live.json`.
It contains no credentials, private trajectory text, or encrypted payloads.

These are functional smoke checks, not a general model-quality or latency
comparison. The localhost wire regression establishes the exact no-duplicate
continuation invariant; the live runs establish that the patched path remains
functional with real incremental continuations and compaction.
