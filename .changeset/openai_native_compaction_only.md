---
roder: patch
roder-api: minor
roder-core: patch
roder-app-server: patch
roder-ext-openai-responses: patch
---

# Keep OpenAI Responses compaction provider-owned

Use OpenAI native compaction for automatic, manual, and context-limit recovery,
including models absent from the local catalog. API-key and Codex subscription
requests both use the streamed `compaction_trigger` strategy on Responses.
Preserve the full input window and encrypted boundary with retained messages;
clear parent-turn output constraints during compaction. Propagate native failures
instead of replacing conversation state with Roder text summaries.
