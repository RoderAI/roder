# Roder DeepSeek Platform Provider

Roder exposes DeepSeek Platform as a first-class provider id:

```text
deepseek
```

Display name:

```text
DeepSeek Platform
```

The built-in default model is:

```text
deepseek/deepseek-chat
```

DeepSeek's OpenAI-compatible Chat Completions API is used at:

```text
https://api.deepseek.com/v1
```

Roder sends requests to `POST /chat/completions` with a Bearer API key.

## API Key Setup

Create or copy a key from the [DeepSeek Platform](https://platform.deepseek.com/api_keys)
dashboard, then paste it into the TUI provider prompt or configure it with an
environment variable:

```sh
export DEEPSEEK_API_KEY="..."
```

Supported key env vars:

```text
DEEPSEEK_API_KEY
RODER_DEEPSEEK_API_KEY
```

The TUI stores keys through `providers/configure` under:

```toml
[providers.deepseek]
api_key = "..."
```

Roder never reads `OPENAI_API_KEY` for DeepSeek, so credentials cannot be sent
to the wrong provider by accident. The provider is listed even without a key so
the TUI and app-server can show setup state; a turn started without a key fails
locally with setup guidance and makes no HTTP request.

## Optional Config

Override the endpoint only for local testing or a DeepSeek-compatible deployment:

```toml
provider = "deepseek"
model = "deepseek-chat"

[providers.deepseek]
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
```

Supported base URL env vars:

```text
DEEPSEEK_BASE_URL
RODER_DEEPSEEK_BASE_URL
```

## Models

Roder ships these built-in DeepSeek Platform models offline:

| Model id            | Display name       | Notes                                      |
| ------------------- | ------------------ | ------------------------------------------ |
| `deepseek-chat`     | DeepSeek Chat      | Default non-thinking chat alias            |
| `deepseek-reasoner` | DeepSeek Reasoner  | Thinking/reasoner alias                    |
| `deepseek-v4-flash` | DeepSeek V4 Flash  | Fast coding/chat model                     |
| `deepseek-v4-pro`   | DeepSeek V4 Pro    | Higher-capability coding/reasoning model   |

Use labels such as:

- `deepseek/deepseek-chat`
- `deepseek/deepseek-reasoner`
- `deepseek/deepseek-v4-flash`
- `deepseek/deepseek-v4-pro`

Model ids are preserved exactly on the wire.

## Requests

Roder routes DeepSeek through the shared OpenAI Chat Completions transport:

```text
POST /chat/completions
Authorization: Bearer <DEEPSEEK_API_KEY>
```

Example request body shape:

```json
{
  "model": "deepseek-chat",
  "stream": true
}
```

## Docs

Upstream API reference: https://api-docs.deepseek.com/
