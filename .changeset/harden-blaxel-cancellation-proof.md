---
roder-ext-runner-blaxel: patch
---

# Harden Blaxel cancellation proof

Use exact environment-tag matching with Linux pidfds, retain idempotent
cancellation acknowledgements, and make cleanup generation-safe across retries
and reused command IDs.
