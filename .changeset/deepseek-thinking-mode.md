---
roder-api: patch
roder-core: patch
roder-ext-deepseek: patch
roder-ext-openai-chat-completions: patch
roder: patch
---

# Fix DeepSeek thinking mode reasoning in Ctrl+P and tool rollouts

DeepSeek models advertise real thinking efforts again, stream
`reasoning_content`, send the DeepSeek `thinking` toggle, and pass CoT back on
tool-call turns.
