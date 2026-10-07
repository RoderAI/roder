---
roder-api: patch
roder-core: patch
roder-tools: patch
roder-tui: patch
roder-app-server: patch
---

# Align thread goals with Codex

Protect unfinished goals, support explicit model-requested pause, account in-flight
usage, enforce exhausted budgets, preserve stopped states when editing, and audit
completion and repeated blockers before ending autonomous goal work.
Automatic continuation preserves turn configuration and rechecks goal state
under turn admission to avoid racing a pause or user turn.
Conversation forks inherit the source goal snapshot after flushing usage.
