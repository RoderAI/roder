---
roder: patch
roder-extension-host: patch
---

# Keep the cua desktop tools in the released build

The cua desktop tools are back in the default `base` build, so
`[cua] enabled = true` works with the released binary again. They stay off
until that setting is on. A build without the `cua` feature now fails on
`[cua] enabled = true` instead of ignoring it.
