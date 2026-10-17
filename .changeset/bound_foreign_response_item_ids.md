---
roder-ext-openai-responses: patch
---

# Prevent long history IDs from rejecting follow-up turns after switching providers

Normalize oversized response item IDs before replaying existing or compacted
history to OpenAI or Codex. Stable, bounded IDs preserve tool-call pairing and
WebSocket continuation while leaving saved history and SuperGrok requests intact.
