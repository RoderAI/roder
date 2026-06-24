---
roder-ext-claude-code: patch
---

# Lock in `request_user_input` support on the Claude Code path

Add the regression coverage the changelog already promised but that was missing
from the test suite. The new `tool_loop` test proves the interactive survey tool
is fully wired on the Claude Code path: it is advertised to the CLI as
`mcp__roder__request_user_input`, the `can_use_tool` callback pre-authorizes it,
and calling it routes through Roder's `TurnToolExecutor` with the nested
`questions` payload preserved through input repair (so the survey reaches the
runtime tool intact instead of being flattened by `retain_schema_properties`).
The runtime executor blocks the call until the client answers and returns the
resolved answers to the model.
