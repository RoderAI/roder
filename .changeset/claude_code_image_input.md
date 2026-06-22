---
roder-ext-claude-code: patch
---

# Deliver image input to the Claude Code provider as real image blocks

The Claude Code provider advertised `image_input: true` but only ever sent the
transcript as a plain text string, so any image attached to a user message was
serialized via `format!("{item:?}")` — i.e. the base64 `data:` URL was dumped
into the prompt as text rather than passed to the model as an image.

The `claude-code-sdk-rust` SDK now exposes a `UserMessageInput` (text or a list
of `InputContentBlock`s, including base64/URL `ImageSource` image blocks) and
its streaming/query entrypoints accept it. The provider decodes each
`UserMessage` image data URL into a real image content block and only replays
the message text (without the raw base64 bytes) in the prompt, so multimodal
turns reach Claude correctly.
