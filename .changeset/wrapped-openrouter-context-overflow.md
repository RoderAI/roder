---
roder-ext-openai-responses: patch
---

# Recognise upstream context overflow wrapped by OpenRouter

Decode canonical context-limit codes from the JSON string in an HTTP 400 error's `metadata.raw`. Preserve the upstream code and classify it as `ContextWindowExceeded` so hosts can compact and retry. Explicit outer error codes, malformed payloads, and other HTTP status policies keep their existing behavior.
