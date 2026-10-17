---
roder-ext-openai-responses: patch
---

# Fix switching SuperGrok threads to OpenAI or Codex

Remove the incompatible reasoning status field when replaying history to OpenAI
or Codex, including restored compacted input. Preserve saved thread history and
the original fields when continuing with SuperGrok.
