---
roder-core: patch
roder-ext-kimi-code: patch
roder-ext-openai-chat-completions: patch
---

# Kimi Code OAuth chat requests omit unsupported OpenAI-compat fields

OAuth turns on `api.kimi.com/coding/v1` no longer send `stream_options` or
`parallel_tool_calls`, which caused 400 responses on the managed Kimi Code API.
Adds configurable flags on the shared chat-completions helper and gates
`should_compact_transcript` to test builds only.