---
roder-tools: patch
---

# Allow create_goal after a completed goal

`create_goal` only fails while an active goal is in progress. Completed, blocked, paused, or limited goals can be replaced so resumed sessions can start the next objective.
