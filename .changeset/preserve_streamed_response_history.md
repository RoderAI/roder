---
roder: patch
roder-ext-openai-responses: patch
---

# Prevent duplicate model history in Responses continuations

Keep completed streamed output when the terminal response omits its output
items. WebSocket continuations now account for reasoning, assistant messages,
and tool calls already held by the provider instead of resending them alongside
`previous_response_id`.
