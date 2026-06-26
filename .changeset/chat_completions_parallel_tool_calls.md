---
roder-ext-openai-chat-completions: patch
---

# Fix parallel tool calls on chat-completions providers (Kimi Code, etc.)

Coalesce a turn's parallel tool calls into a single assistant `tool_calls`
message followed by one `role: tool` message per id. Previously each tool call
was emitted as its own assistant message, so a turn with multiple tool calls
(e.g. several `write_file` calls at once) produced an invalid request and the
provider returned `400 ... tool_call_ids did not have response messages`.
