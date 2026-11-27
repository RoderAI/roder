---
roder: patch
roder-tui: patch
roder-ext-chrome: patch
---

# Keep Chrome pairing stable across Roder instances

Automatically assigned fallback ports on secondary Roder instances no longer
replace the saved browser-pairing endpoint. Explicitly pinned listeners retain
their existing endpoint-selection behavior.
Desktop browser tools identify an empty browser explicitly and create a real new
tab on CDP backends that support it. Electron's unsupported tab-creation response
now explains how to open the Browser pane or connect the Chrome extension.
