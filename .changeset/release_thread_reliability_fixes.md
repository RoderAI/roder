---
roder: patch
---

# Make model switching and follow-up turns more reliable

Ship the fixes for oversized history IDs and incompatible reasoning fields when
switching from SuperGrok to OpenAI or Codex. Avoid full-history loads during model
selection, show rejected follow-up requests, and retain failed input for `/retry`
instead of leaving users without feedback.
