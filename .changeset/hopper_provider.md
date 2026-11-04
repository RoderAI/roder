---
roder-ext-hopper: minor
roder-api: minor
roder-extension-host: minor
roder: minor
roder-tui: patch
---

# Add Hopper inference provider

Adds a first-class `hopper` provider for [Hopper](https://withhopper.com) using
its OpenAI-compatible Chat Completions API at `https://api.withhopper.com/v1`,
with `HOPPER_API_KEY` auth and built-in models `deepseek-v4.1-flash-1017`
(default) and `deepseek-v4.1-flash-1017-abliterated`: 1M-token context, tool calls and
DeepSeek thinking-mode efforts from `none` to `max`.
