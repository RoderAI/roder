---
roder-tools: patch
---

# Allow glob to search absolute paths outside the workspace

Match `glob` behavior with the other file search tools by allowing absolute patterns to search directly when filesystem access is unrestricted, while preserving workspace-only scoping when configured.
