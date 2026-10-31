---
roder-ext-jev: patch
---

# Install Jev's observation scripts once per document, and keep freshness guards as digests

Each document now installs `snapshot.js` and `context.js` once, under a
read-only global with a random per-process key. The scripts are registered
with `Page.addScriptToEvaluateOnNewDocument`, and an older document installs
them lazily on its first read. Each observation and freshness check then
sends a short call (about 660 bytes) instead of about 70 KB of script.

Guards keep their structured fields but hold a digest of the text around
the control. The page key and freshness marker hold digests of the form
state, title, page text, disabled controls and on-screen actions, so stale
diagnostics still name the field that changed.
