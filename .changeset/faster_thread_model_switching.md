---
roder-core: patch
roder-app-server: patch
---

# Avoid loading conversation history when switching models

Read and update thread model selection through the metadata-only store API
instead of loading and reconstructing the entire conversation. This avoids
history-sized work when switching models or resolving a thread's selection.
