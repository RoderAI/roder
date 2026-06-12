# Roder Cloud Provider

Roder exposes the roder.cloud hosted service as a first-class provider id:

```text
roder-cloud
```

The default model is the free hosted alias:

```text
roder.cloud/free
```

The built-in catalog also lists the other hosted aliases (`roder.cloud/openai/gpt-5.5`, `roder.cloud/anthropic/claude-opus-4-7`, `roder.cloud/google/gemini-3.1-pro-preview`). Team-enabled models — including BYOK aliases like `openai/gpt-5.5` — appear automatically once a key is configured, because the model list comes from the service.

## Authentication

roder.cloud uses two credentials:

1. A long-lived team API key with the `roder_` prefix, created once in the dashboard at `https://roder.cloud` (Team -> API keys; the secret is shown once).
2. A short-lived JWT that Roder mints automatically by exchanging the API key at `POST /api/v1/inference_tokens`. The JWT lives ~10 minutes; Roder caches it and refreshes before expiry or after an `invalid_token` rejection. The API key itself is never sent to the inference edge.

The guided setup validates the key with one real token exchange before saving it to user config:

```sh
roder auth login roder-cloud
roder auth status roder-cloud
roder auth logout roder-cloud
```

Or configure it directly:

```sh
export RODER_CLOUD_API_KEY="roder_..."
```

Supported key env vars:

```text
RODER_CLOUD_API_KEY
RODER_CLOUD_TOKEN
```

The TUI stores keys through `providers/configure` under:

```toml
[providers.roder-cloud]
api_key = "roder_..."
```

## Endpoints

The inference edge URL is deploy-specific and must be configured (it is not hardcoded). Include the `/v1` suffix:

```toml
provider = "roder-cloud"
model = "roder.cloud/free"

[providers.roder-cloud]
base_url = "https://<inference-host>/v1"
api_key_env = "RODER_CLOUD_API_KEY"
```

Env equivalents:

```text
RODER_CLOUD_BASE_URL   # inference edge incl /v1 (local dev: http://127.0.0.1:8080/v1)
RODER_CLOUD_WEB_URL    # dashboard/token-exchange host (default https://roder.cloud)
```

## Requests and Limitations

roder.cloud speaks a synchronous subset of the OpenAI Responses API:

- `POST {base_url}/responses` returns one completed JSON body; there is no SSE. Roder synthesizes its stream events from the completed response.
- `input` is sent as a flat string (`role: text` lines, matching the godex `/v1/messages` shim) because the edge forwards it verbatim to heterogeneous upstream routes that do not all accept structured Responses input items.
- Tool calls are not supported: the edge does not round-trip function-call payloads, so the provider advertises `tool_calls: false` and prunes `tools` from requests.
- `background`, reasoning controls, prompt-cache keys, and structured output are not part of the subset and are never sent.
- `GET {base_url}/models` returns the team-filtered model list used by the model picker.

Quota errors surface as actionable messages: `quota_exceeded` (HTTP 429, monthly request/token limits) and `model_not_allowed` (HTTP 403, enable the model for your team in the dashboard).

## Live Checks

Offline tests use fake HTTP servers for both the token exchange and the inference edge. Live checks are opt-in:

```sh
RODER_RODER_CLOUD_LIVE=1 \
RODER_CLOUD_API_KEY="roder_..." \
RODER_CLOUD_BASE_URL="http://127.0.0.1:8080/v1" \
RODER_CLOUD_WEB_URL="http://localhost:3000" \
cargo test -p roder-ext-roder-cloud --test live_roder_cloud -- --ignored
```

Against a local godex stack, run `bin/dev` in the godex repo first and create a team API key from the dashboard.
