---
roder-app-server: patch
roder: patch
---

# Report failed native turns as ACP prompt errors

Return a JSON-RPC internal error with native failure details instead of a
successful end-turn response when a provider or runtime turn fails.
Wait for native turn teardown before settling an ACP prompt so an immediate
follow-up cannot be steered into an already-finished turn.
