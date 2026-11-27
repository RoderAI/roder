---
roder-ext-jev: patch
---

# Remove unused `today` re-export from the Jev text helper

Import `today` directly in the text helper tests instead of re-exporting it,
fixing an `unused_imports` warning in release builds.
