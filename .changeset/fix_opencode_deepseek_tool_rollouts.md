---
roder-api: patch
roder-ext-opencode: patch
roder: patch
---

# Fix OpenCode DeepSeek multi-step tool rollouts

Refresh the OpenCode Zen model catalog (drop disabled free DeepSeek IDs, add
current free models and paid `deepseek-v4-flash` / `deepseek-v4-pro`), coalesce
parallel tool calls into valid chat-completions histories for longer DeepSeek
rollouts, and surface clearer OpenCode ModelError/CreditsError messages.
