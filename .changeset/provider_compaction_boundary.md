---
roder-core: patch
roder-ext-openai-responses: patch
roder-ext-anthropic: patch
---

# Honor provider compaction boundaries

Drop pre-compaction history after OpenAI server-side compaction items so long sessions no longer re-send and re-compact the full window every request. Treat provider compaction as a local transcript boundary, and map Anthropic local context summaries correctly when the emergency client path runs.
