---
roder-api: minor
roder-ext-openai-responses: patch
roder: patch
---

# Refresh the OpenRouter model catalog and stop assuming server-side compaction

OpenRouter now ships 25 catalogued routes (Kimi K3, Claude 5.5 family, GPT-6.x,
Gemini 3.7/3.8 Flash, Grok 4.6/4.7, DeepSeek V4, Qwen 3.8, GLM 5.3, MiMo 2.6,
Mistral Large 4, Muse Spark) with the context windows, image support, and
reasoning efforts reported by the live `/models` listing. OpenRouter accepts
the OpenAI `context_management` field but does not compact server-side, so the
OpenRouter entries (including Grok 4.6, which previously claimed it) now compact
client-side at 90% of the window instead of overflowing. Routes that are only
discovered at runtime now offer reasoning efforts when OpenRouter reports that
they support reasoning.
