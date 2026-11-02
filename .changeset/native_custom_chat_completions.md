---
roder: minor
roder-config: minor
roder-extension-host: minor
roder-core: patch
roder-ext-openai-chat-completions: patch
---

# Support native Chat Completions for custom inference providers

Custom endpoints can use Roder's native Rust Chat Completions transport with
`protocol = "chat_completions"`, including streaming answers and tool-call
round trips. Optional model IDs, deployed context-window limits, and tool-schema
allowlists make small-context endpoints easier to configure. Existing custom
Responses providers retain their defaults.
