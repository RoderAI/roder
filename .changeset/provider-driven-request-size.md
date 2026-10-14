---
roder-ext-openai-responses: patch
roder: patch
---

# Let providers judge request size and recognise OpenRouter context overflow

Responses requests are sent exactly as built: the 15 MB client-side budget (`RODER_RESPONSES_MAX_REQUEST_BYTES`) and its shedding of older tool images are removed, so history is never rewritten on the wire and the provider prompt cache stays valid. HTTP 413 maps to `RequestTooLarge`, and OpenRouter's over-long prompt error (HTTP 400 with only a "maximum context length" message) maps to `ContextWindowExceeded`, so the runtime compacts in response instead of failing the turn.
