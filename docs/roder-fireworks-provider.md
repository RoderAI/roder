# Roder Fireworks Provider

Roder exposes Fireworks AI as a first-class inference provider id:

```text
fireworks
```

The provider uses Fireworks-specific API-key configuration and the Fireworks OpenAI-compatible Responses endpoint. Model ids are Fireworks account-scoped ids and must be preserved exactly.

## API Key Setup

Configure Fireworks with either environment variable:

```sh
export FIREWORKS_API_KEY="..."
export RODER_FIREWORKS_API_KEY="..."
```

Or store the key reference in Roder config:

```toml
provider = "fireworks"
model = "accounts/fireworks/models/qwen3-235b-a22b"

[providers.fireworks]
api_key_env = "FIREWORKS_API_KEY"
base_url = "https://api.fireworks.ai/inference/v1"
```

Roder deliberately does not read `OPENAI_API_KEY` or `OPENAI_API_BASE` for Fireworks. This prevents OpenAI credentials from being sent to Fireworks by accident. Raw Fireworks keys must stay out of checked-in files, docs, fixtures, transcripts, and roadmap evidence.

Create and manage Fireworks API keys from the Fireworks dashboard or CLI documented by Fireworks:

```text
https://fireworks.ai/account/api-keys
```

## Model IDs

Fireworks models use account-scoped ids such as:

```text
accounts/fireworks/models/qwen3-235b-a22b
accounts/fireworks/models/deepseek-v3p1
accounts/<account-id>/deployments/<deployment-id>
```

Use provider/model notation by keeping the provider separate from the model:

```text
fireworks/accounts/fireworks/models/qwen3-235b-a22b
```

The provider is `fireworks`; the model is the full remaining `accounts/...` string. Clients must not split the model id on every slash.

## Transport

The first implementation uses:

```text
POST https://api.fireworks.ai/inference/v1/responses
GET  https://api.fireworks.ai/inference/v1/models
```

Responses requests always include:

```json
{
  "store": false,
  "stream": true
}
```

Roder owns transcript storage, so Fireworks provider-side conversation storage is disabled by default. Roder also does not rely on `previous_response_id`; every turn sends the canonical transcript context needed for that request.

Function tools remain client-executed through Roder's normal tool registry, policy, hooks, and transcript events. Fireworks server-executed MCP/SSE tools are not enabled by this provider.

## Discovery And Offline Behavior

`providers/list` exposes Fireworks even without credentials. In that state it reports API-key auth as unauthenticated and returns the built-in fallback model:

```text
accounts/fireworks/models/qwen3-235b-a22b
```

When credentials are available, model discovery runs in the background against `GET <base_url>/models` and caches successful responses in the normal model cache. Provider/model picker calls stay responsive by returning cached or built-in models instead of blocking on network discovery.

## Errors

Provider errors are mapped to actionable Roder messages for missing or invalid keys, billing/quota issues, unavailable or undeployed models, oversized payloads, request timeouts, rate limits/capacity, malformed parameters, and retryable upstream failures. Error messages do not include bearer headers or raw credential-bearing request data.

## Live Smoke

Default tests are offline and use fake HTTP or catalog data. A live smoke is available only when explicitly enabled:

```sh
RODER_FIREWORKS_LIVE=1 \
FIREWORKS_API_KEY=... \
FIREWORKS_LIVE_MODEL=accounts/fireworks/models/qwen3-235b-a22b \
cargo test -p roder-ext-fireworks --test live_fireworks -- --ignored
```

Do not paste live keys, raw provider responses, private prompts, or request ids into docs, fixtures, transcripts, or roadmap evidence.
