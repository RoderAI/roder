---
roder-automations: patch
roder-code-index: patch
roder-ext-memory: patch
---

# Publish SQLite dependency alignment for registry builds

Release the automations, code index, and memory packages with the workspace's
rusqlite 0.41 dependency. Their previous published versions still require
rusqlite 0.40, which conflicts with the native SQLite linkage used by the
new usage-analytics package.

This makes the existing dependency refresh available to crates.io consumers.
No runtime behavior or previously published tags are changed.
