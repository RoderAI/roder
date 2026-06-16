# Roder OpenCode Providers

Roder exposes OpenCode as two provider ids so model provenance is clear in the model picker:

```text
opencode
opencode-go
```

Model labels use `<provider>/<model>`, for example:

```text
opencode/gpt-5.5
opencode/big-pickle
opencode-go/kimi-k2.6
opencode-go/qwen3.6-plus
```

Kimi K2 models are also available directly via the dedicated `kimi-code` provider (`kimi-code/kimi-for-coding`), which authenticates against your Kimi Code subscription through device OAuth (managed API) or a Moonshot Open Platform API key. See `docs/roder-kimi-code-provider.md`.

## API Key Setup

OpenCode is API-key based in Roder. Open the OpenCode auth page, create or copy an API key, then paste it into the TUI provider prompt:

```text
https://opencode.ai/auth
```

The TUI stores keys through `providers/configure` under the selected provider:

```toml
[providers.opencode]
api_key = "..."

[providers.opencode-go]
api_key = "..."
```

You can also configure it with an environment variable:

```sh
export OPENCODE_API_KEY="..."
```

Supported key env vars:

```text
OPENCODE_API_KEY
OPENCODE_ZEN_API_KEY
RODER_OPENCODE_API_KEY
OPENCODE_GO_API_KEY
RODER_OPENCODE_GO_API_KEY
```

## Optional Config

Override the endpoint only for local testing or an OpenCode-compatible deployment:

```toml
[providers.opencode]
base_url = "https://opencode.ai/zen/v1"
api_key_env = "OPENCODE_API_KEY"
project_id = "workspace-or-project-id"

[providers.opencode-go]
base_url = "https://opencode.ai/zen/go/v1"
api_key_env = "OPENCODE_GO_API_KEY"
```

Supported base URL env vars:

```text
RODER_OPENCODE_BASE_URL
OPENCODE_BASE_URL
OPENCODE_ZEN_BASE_URL
RODER_OPENCODE_GO_BASE_URL
OPENCODE_GO_BASE_URL
```

## Model Cache

Provider and model picker calls must not block on OpenCode. Roder returns cached models immediately when available, falls back to the built-in OpenCode catalog when the cache is missing, and refreshes stale or missing model data in the background. The cache is keyed by provider id and base URL, so `opencode` and `opencode-go` refresh independently.

The background refresh calls OpenAI-style model discovery:

```text
GET /models
```

Successful discovery writes a redacted cache to:

```text
~/.roder/models-cache.json
```

The default TTL is 6 hours. Set `RODER_MODELS_CACHE_TTL_SECONDS=0` to refresh all provider model caches on every provider-list access, `RODER_MODELS_REFRESH=1` for a manual refresh trigger, or `RODER_MODELS_CACHE_PATH=/path/to/models-cache.json` to override the cache file for diagnostics. The older OpenCode-specific `RODER_OPENCODE_MODELS_CACHE_TTL_SECONDS` and `RODER_OPENCODE_MODELS_REFRESH` names are still accepted. Refresh is still backgrounded; Ctrl+P and provider menus continue to render from cache or built-ins without waiting for the network.

Custom OpenAI-compatible providers configured under `[providers.<id>]` use the same cache path and TTL controls. For those providers, Roder tries both `GET <base_url>/models` and `GET <base_url>/v1/models` before leaving the existing cache unchanged.

## Requests

Inference uses OpenCode's OpenAI-compatible streaming chat completions endpoint at the configured base URL. Roder requests token, tool-call, and usage chunks with `stream: true` and sends OpenCode request headers when values are available:

```text
x-opencode-session
x-opencode-request
x-opencode-client: roder
User-Agent: roder/<version>
```

`x-opencode-project` is sent only when `project_id` or `project_id_env` is explicitly configured. API keys are never logged by the provider.
