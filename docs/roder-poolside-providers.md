# Roder Poolside Provider

Roder exposes Poolside as a first-class provider id:

```text
poolside
```

Model labels use the Poolside model ids directly:

```text
poolside/laguna-m.1
poolside/laguna-xs.2
```

## API Key Setup

Poolside is API-key based in Roder. Open the Poolside API keys page, create or copy an API key, then paste it into the TUI provider prompt:

```text
https://platform.poolside.ai/api-keys
```

The TUI stores keys through `providers/configure` under the selected provider:

```toml
[providers.poolside]
api_key = "..."
```

You can also configure it with an environment variable:

```sh
export POOLSIDE_API_KEY="..."
```

Supported key env vars:

```text
POOLSIDE_API_KEY
RODER_POOLSIDE_API_KEY
```

## Optional Config

Override the endpoint only for local testing or a deployment with the same OpenAI-compatible API:

```toml
[providers.poolside]
base_url = "https://inference.poolside.ai/v1"
api_key_env = "POOLSIDE_API_KEY"
```

Supported base URL env vars:

```text
RODER_POOLSIDE_BASE_URL
POOLSIDE_BASE_URL
```

## Requests

Inference uses Poolside's OpenAI-compatible streaming chat completions endpoint at the configured base URL:

```text
POST /chat/completions
```

Roder requests token, tool-call, and usage chunks with:

```json
{
  "stream": true,
  "stream_options": {
    "include_usage": true
  }
}
```

Poolside supports OpenAI-compatible tool calling, so Roder sends function tools using the standard Chat Completions `tools`, `tool_choice`, and `parallel_tool_calls` fields. Poolside thinking is controlled with `chat_template_kwargs.enable_thinking`, not a reasoning-effort field. Roder enables thinking by default for Laguna models, sends `true` for enabled reasoning selections, and sends `false` when the selected reasoning level is `none`:

```json
{
  "chat_template_kwargs": {
    "enable_thinking": true
  }
}
```

When Poolside returns `reasoning_content`, Roder renders it as reasoning output separately from final assistant content.
