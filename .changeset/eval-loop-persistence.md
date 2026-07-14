---
roder-core: minor
roder-config: minor
roder: minor
---

# Eval loop persistence: continue past tool-failure limit and nudge on empty finalization

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
