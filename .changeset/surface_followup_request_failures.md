---
roder-tui: patch
---

# Show rejected follow-up requests instead of silently discarding errors

Report background turn-start and steering request failures in the transcript,
retain the latest follow-up for `/retry`, and clear a stale working indicator
when the backend confirms that the exact turn being steered has already ended.
Late responses never reactivate completed turns or clear a newer active turn.
