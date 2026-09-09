---
roder-api: minor
roder-ext-anthropic: patch
---

# Add GPT-6 Astra, Gemini 3.8 Flash, and Claude Fable 5.1

Expose `gpt-6-astra` (1.05M context, efforts up to `max`) on the OpenAI/Codex
catalog, `gemini-3.8-flash` on both the Gemini API and Vertex AI providers, and
`claude-fable-5-1` on the direct Anthropic provider and through the local Claude
Code harness. Provider defaults are unchanged.

Fable 5.1 rejects forced tool choice, so the Anthropic request mapping now sends
`tool_choice: auto` for that model instead of `any`/`tool`.
