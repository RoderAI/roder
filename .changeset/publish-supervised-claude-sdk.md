---
roder-ext-claude-code: patch
---

# Consume the portable supervised Claude Code SDK release

Depend on `claude-code-sdk-rust` 0.4.1 so published Roder consumers receive the
same cancellation-safe subprocess cleanup API as the repository build, without
relying on a workspace-local Cargo patch.
