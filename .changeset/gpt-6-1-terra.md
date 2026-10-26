---
roder-api: minor
roder-app-server: patch
roder-ext-openai-responses: patch
roder-ext-jev: patch
roder: patch
---

# Add GPT-6.1 Terra and retire GPT-6 Sol

Expose `gpt-6.1-terra` with a 1M context window and standard prices of $1 / $0.05 / $5 per 1M tokens. The default GPT model is now `gpt-6.1-sol`. `gpt-6-sol` stays resolvable but is hidden from selectors.
