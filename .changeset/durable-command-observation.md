---
roder-api: minor
roder-ext-runner-blaxel: minor
---

Add explicit durably identified command execution and read-only process metadata
observation. Blaxel derives stable process names from a journaled command ID for
original explicit-workspace sessions. No observation authorizes start replay;
unsupported providers refuse durable operations. Metadata excludes process logs.
