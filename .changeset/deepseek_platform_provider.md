---
roder-api: minor
roder-extension-host: minor
roder-ext-deepseek: minor
roder: minor
roder-tui: patch
---

# Add DeepSeek Platform inference provider

Adds first-class `deepseek` provider support labeled "DeepSeek Platform", using
DeepSeek's OpenAI-compatible Chat Completions API at `https://api.deepseek.com/v1`
with `DEEPSEEK_API_KEY` auth and built-in models `deepseek-chat`,
`deepseek-reasoner`, `deepseek-v4-flash`, and `deepseek-v4-pro`.
