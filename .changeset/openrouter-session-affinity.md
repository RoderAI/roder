---
roder-ext-openai-responses: patch
roder: patch
---

# Pin OpenRouter conversations to one provider for prompt caching

OpenRouter requests now always carry a sticky-routing key: the runtime prompt cache key, or the turn's thread id when none is set, sent as `prompt_cache_key` and as `session_id` (truncated to OpenRouter's 256-character limit). OpenRouter then routes every turn of a conversation to the same upstream provider from the first request instead of scattering early turns across cold caches.
