---
roder-tools: patch
roder: patch
---

# Clean up active exec commands when the runtime shuts down

Keep ownership of detached command groups in their wait tasks so graceful app-server shutdown kills unfinished commands and their descendants. Commands that already finished normally retain any background processes they started.
