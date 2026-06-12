---
roder-ext-cursor: patch
---

# Prevent Cursor protobuf decoder panics on malformed payloads

Reject overlong varints and out-of-bounds protobuf fields as decode errors so
unexpected Cursor frames do not stop the agent turn.
