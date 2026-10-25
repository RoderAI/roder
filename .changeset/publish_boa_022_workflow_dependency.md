---
roder-dynamic-workflows: patch
---

# Publish the Boa dependency refresh for registry consumers

Release dynamic workflows with the workspace's Boa 0.22 dependency. The previous
published package still requires Boa 0.21 and its ICU 2.0 dependency, which cannot
resolve with the ICU 2.3 versions used by the current HTTP stack.

This publishes an already-merged dependency refresh and unblocks verified
crates.io builds without changing existing release tags or runtime behavior.
