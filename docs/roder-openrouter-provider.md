# Roder OpenRouter Provider

Roder exposes OpenRouter as a first-class provider id:

```text
openrouter
```

The built-in offline fallback model is Grok Build 0.1 through OpenRouter:

```text
openrouter/x-ai/grok-build-0.1
```

Direct xAI uses the model id `grok-build-0.1`; OpenRouter uses the provider-prefixed slug `x-ai/grok-build-0.1`. Roder preserves the OpenRouter slug exactly.

## API Key Setup

OpenRouter is API-key based in Roder. Create or copy a key from the OpenRouter dashboard, then paste it into the TUI provider prompt or configure it with an environment variable:

```sh
export OPENROUTER_API_KEY="..."
```

Supported key env vars:

```text
OPENROUTER_API_KEY
RODER_OPENROUTER_API_KEY
```

The TUI stores keys through `providers/configure` under:

```toml
[providers.openrouter]
api_key = "..."
```

## Optional Config

Override the endpoint only for local testing or an OpenRouter-compatible deployment:

```toml
provider = "openrouter"
model = "x-ai/grok-build-0.1"

[providers.openrouter]
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
http_referer = "https://example.com"
app_title = "Roder"
```

Supported base URL env vars:

```text
OPENROUTER_BASE_URL
RODER_OPENROUTER_BASE_URL
```

Optional attribution header env vars:

```text
OPENROUTER_HTTP_REFERER
RODER_OPENROUTER_HTTP_REFERER
OPENROUTER_APP_TITLE
RODER_OPENROUTER_APP_TITLE
```

Roder only sends attribution headers when configured.

## Model Discovery

Roder returns built-in OpenRouter models immediately, refreshes stale model data in the background, and caches successful discovery in `~/.roder/models-cache.json`.

OpenRouter discovery uses:

```text
GET /models
```

The cache is keyed by provider id and base URL. Set `RODER_MODELS_CACHE_TTL_SECONDS=0` to refresh all provider model caches on every provider-list access, `RODER_MODELS_REFRESH=1` for a manual refresh trigger, or `RODER_MODELS_CACHE_PATH=/path/to/models-cache.json` to override the cache file for diagnostics.

## Requests

Roder routes OpenRouter through the OpenAI Responses-compatible transport at:

```text
POST /responses
```

Requests send the selected OpenRouter model slug exactly, for example:

```json
{
  "model": "x-ai/grok-build-0.1",
  "stream": true
}
```

Roder does not send OpenAI encrypted reasoning replay fields to OpenRouter by default. Live checks are opt-in:

```sh
RODER_OPENROUTER_LIVE=1 OPENROUTER_LIVE_MODEL=x-ai/grok-build-0.1 cargo test -p roder-ext-openrouter --test live_openrouter -- --ignored
```
