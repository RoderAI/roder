---
roder-ext-openai-chat-completions: major
roder-ext-deepseek: patch
roder-ext-hopper: patch
roder-ext-kimi-code: patch
roder-ext-synthetic: patch
roder-ext-xiaomi-mimo: patch
roder-extension-host: patch
roder-app-server: patch
roder-dist-hosted: patch
roder: patch
roder-tui: patch
---

# Forward images through Chat Completions

Preserve user image URLs and inline base64 images in both Chat Completions
transports, and replay validated tool-result images after the complete batch
of tool responses in the shared streaming transport. Enable image input for
Hopper's built-in models.

`ChatMessage.content` is now a JSON value rather than a Rust string so it can
represent either text or multimodal content parts. Text-only wire messages
remain unchanged. Release compatible reverse dependencies together.
