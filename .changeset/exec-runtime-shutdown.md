---
roder-tools: patch
roder: patch
---

# Clean up active Unix exec commands when the runtime shuts down

Keep ownership of detached command groups in their wait tasks so graceful app-server shutdown on macOS and Linux kills unfinished commands and their descendants. Commands that already finished normally retain any background processes they started. Windows gains direct-child cleanup on drop; descendant cleanup is not guaranteed there.
