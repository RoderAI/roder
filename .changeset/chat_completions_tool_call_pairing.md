---
roder-ext-openai-chat-completions: patch
---

# Always pair tool calls with results in chat-completions requests

Harden the chat-completions message builder so an assistant `tool_calls` message
is ALWAYS immediately followed by exactly one `role: tool` message per id. Each
coalesced assistant message now emits its tool results right after it (looked up
by id, or a placeholder when none was recorded), and orphan tool results with no
matching call are dropped. This makes the request structurally valid even when a
result is missing or out of order (e.g. dropped during context compaction or a
partial/replayed transcript), fixing recurring `400 ... tool_call_ids did not
have response messages` errors on parallel tool calls (Kimi Code and other
OpenAI-compatible providers).
