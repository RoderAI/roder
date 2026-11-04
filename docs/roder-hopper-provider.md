# Roder Hopper Provider

Roder exposes [Hopper](https://withhopper.com) as a first-class provider id:

```text
hopper
```

Display name:

```text
Hopper
```

The built-in default model is:

```text
hopper/deepseek-v4.1-flash-1017
```

Hopper's OpenAI-compatible Chat Completions API is used at:

```text
https://api.withhopper.com/v1
```

Roder sends requests to `POST /chat/completions` with a Bearer API key.

## API Key Setup

Create a key in the [Hopper console](https://withhopper.com/console/keys), then
paste it into the TUI provider prompt or configure it with an environment
variable:

```sh
export HOPPER_API_KEY="sk_hopper_..."
```

Supported key env vars:

```text
HOPPER_API_KEY
RODER_HOPPER_API_KEY
```

The TUI stores keys through `providers/configure` under:

```toml
[providers.hopper]
api_key = "..."
```

Roder never reads `OPENAI_API_KEY` for Hopper, so credentials cannot be sent to
the wrong provider by accident. The provider is listed even without a key so
the TUI and app-server can show setup state; a turn started without a key fails
locally with setup guidance and makes no HTTP request.

## Optional Config

```toml
provider = "hopper"
model = "deepseek-v4.1-flash-1017"

[providers.hopper]
base_url = "https://api.withhopper.com/v1"
api_key_env = "HOPPER_API_KEY"
```

Supported base URL env vars:

```text
HOPPER_BASE_URL
RODER_HOPPER_BASE_URL
```

## Models

| Model id                               | Display name                         | Notes                               |
| -------------------------------------- | ------------------------------------ | ----------------------------------- |
| `deepseek-v4.1-flash-1017`             | DeepSeek V4.1 Flash 1017             | Default; October 17, 2026 snapshot  |
| `deepseek-v4.1-flash-1017-abliterated` | DeepSeek V4.1 Flash 1017 Abliterated | Same snapshot with refusals ablated |

Both models have a 1M-token context window, tool calls and thinking mode
(default effort `high`). Model ids are preserved exactly on the wire.

## Thinking mode

Hopper serves DeepSeek with DeepSeek's thinking-mode wire format, controlled
from the Ctrl+P reasoning menu:

| Roder effort | Wire behavior |
| --- | --- |
| `none` | `thinking: { "type": "disabled" }` |
| `low` / `high` / `xhigh` / `max` | `thinking: { "type": "enabled" }` + `reasoning_effort` |

Streaming CoT arrives as `delta.reasoning_content` and is shown as reasoning in
the TUI. On tool-call turns Roder passes it back as `reasoning_content` on the
assistant `tool_calls` message.

## Docs

Hopper API docs: https://docs.withhopper.com
