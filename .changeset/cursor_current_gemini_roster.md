---
roder-api: patch
roder-ext-cursor: patch
---

# Refresh Cursor Gemini models

Add Gemini 4 Argon with a 1M-token context window and high effort by default.
Send its registry-supported reasoning_effort parameter instead of generic legacy
parameters that Cursor rejects for this model.
Replace Cursor's stale Gemini 3.1 Pro preview id with gemini-3.1-pro and prevent
preview ids from returning through discovery or older caches.
