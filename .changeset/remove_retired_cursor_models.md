---
roder-api: patch
roder-ext-cursor: patch
---

# Remove retired Cursor models from model selection

Remove Cursor Grok 4.3, which the service rejects as deprecated. Prevent retired
Cursor models and their effort/fast variants from reappearing through stale
caches or live discovery, including the already-retired GPT-5.5 variants.
