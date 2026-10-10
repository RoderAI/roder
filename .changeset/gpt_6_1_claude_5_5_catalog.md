---
roder-api: minor
roder-ext-openai-responses: patch
---

# Add GPT-6.1-Sol, Claude Sonnet 5.5, and Claude Haiku 5.5; retire GPT-5.x catalog entries

Offer GPT-6.1-Sol on the OpenAI and Codex routes, and Claude Sonnet 5.5 and Claude Haiku 5.5 on both the Anthropic and Claude Code routes.

Retire the GPT-5.x catalog entries (GPT-5.6, GPT-5.5, GPT-5.5-Fast, GPT-5.4, GPT-5.4-Mini, GPT-5.3-Codex-Spark, and their OpenCode, Cursor, and Roder Cloud routes). They are hidden from model lists but still resolve for existing threads, profiles, and compaction policy. The OpenCode default model moves from gpt-5.5 to gpt-6-sol.

Enable provider-native tool search for GPT-6 point releases such as gpt-6.1-sol, which previously matched only the gpt-6-* ids.
