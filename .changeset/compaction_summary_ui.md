---
roder-api: patch
roder-core: patch
roder-app-server: patch
roder-tui: patch
roder-ext-openai-responses: patch
---

# Fix provider compaction thrashing and show token/duration summaries

Persist OpenAI/Codex compaction items as soon as the stream emits them so a
later SSE decode failure cannot drop the boundary and re-compact every round.
Surface before/after estimated tokens and elapsed time in the TUI and
app-server item stream.
