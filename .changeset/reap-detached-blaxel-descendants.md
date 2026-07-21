---
roder-ext-runner-blaxel: patch
---

# Reap detached Blaxel command descendants

Tag each remote command's process tree and require an untagged sandbox cleanup
process to prove that no tagged descendants remain before cancellation succeeds.
