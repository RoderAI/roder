---
roder-ext-jev: minor
---

# Expose an injectable JEV engine

Add public browser, decision-client, and text-value interfaces around the
bounded JEV loop. Hosted runners can now retain browser supervision, evidence
capture, and secret indirection while reusing JEV's action planner. The
existing `jev_browse` tool continues to use the built-in Chrome and provider
adapters.
