---
roder-tools: patch
roder: patch
---

# Expand Codex-style `user://` tool paths to `$HOME`

File lookup tools resolve skill-style `user://...` paths against the home
directory and `workspace://...` against the workspace root, so agents can open
canonical skill paths instead of treating the scheme as a literal relative path.
