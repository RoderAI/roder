---
roder-core: patch
---

# Fix local hook denials, chaining, and session scope

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
