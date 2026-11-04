# Changelog
## 0.1.1 (2026-11-04)

### Features

#### Add Hopper inference provider

Adds a first-class `hopper` provider for [Hopper](https://withhopper.com) using
its OpenAI-compatible Chat Completions API at `https://api.withhopper.com/v1`,
with `HOPPER_API_KEY` auth and built-in models `deepseek-v4.1-flash-1017`
(default) and `deepseek-v4.1-flash-1017-abliterated`: 1M-token context, tool calls and
DeepSeek thinking-mode efforts from `none` to `max`.

### Fixes

#### Forward images through Chat Completions

Preserve user image URLs and inline base64 images in both Chat Completions
transports, and replay validated tool-result images after the complete batch
of tool responses in the shared streaming transport. Enable image input for
Hopper's built-in models.

`ChatMessage.content` is now a JSON value rather than a Rust string so it can
represent either text or multimodal content parts. Text-only wire messages
remain unchanged. Release compatible reverse dependencies together.
