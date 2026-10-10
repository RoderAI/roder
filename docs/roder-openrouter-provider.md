# Roder OpenRouter Provider

Roder exposes OpenRouter as a first-class provider id:

```text
openrouter
```

The built-in default model is Grok 4.6 through OpenRouter:

```text
openrouter/x-ai/grok-4.6
```

Direct xAI uses the model id `grok-4.6`; OpenRouter uses the provider-prefixed slug `x-ai/grok-4.6`. Roder preserves the OpenRouter slug exactly.

## Built-in Models

OpenRouter lists several hundred routes. Roder ships a curated, tool-capable subset in `crates/roder-api/src/catalog/openrouter.rs` so these work offline and carry the reasoning efforts, image support, and compaction threshold that discovery cannot supply. Context windows come from the live `https://openrouter.ai/api/v1/models` listing.

| Model | Context window | Compacts at |
| --- | ---: | ---: |
| `moonshotai/kimi-k3` | 1,048,576 | 943,718 |
| `moonshotai/kimi-k2.7-code` | 262,144 | 235,929 |
| `moonshotai/kimi-k2.6` | 262,144 | 235,929 |
| `x-ai/grok-4.7`, `x-ai/grok-4.6` | 500,000 | 450,000 |
| `anthropic/claude-opus-5.5`, `-sonnet-5.5`, `-haiku-5.5`, `-fable-5.1`, `-opus-5` | 1,000,000 | 900,000 |
| `openai/gpt-6.1-sol`, `gpt-6-sol`, `gpt-6-luna`, `gpt-6-astra` | 1,050,000 | 945,000 |
| `google/gemini-3.8-flash`, `gemini-3.7-flash` | 1,048,576 | 943,718 |
| `deepseek/deepseek-v4-pro-0813`, `deepseek-v4.1-flash` | 1,048,576 | 943,718 |
| `qwen/qwen3.8-max-0902`, `qwen3.8-flash` | 1,000,000 | 900,000 |
| `z-ai/glm-5.3`, `glm-5.3-flash` | 1,048,576 | 943,718 |
| `xiaomi/mimo-v2.6-pro` | 1,050,000 | 945,000 |
| `mistralai/mistral-large-4-0` | 1,048,576 | 943,718 |
| `meta/muse-spark-1.3` | 1,048,576 | 943,718 |

Any other OpenRouter route is still selectable: discovery supplies its context window, and offers none/low/medium/high reasoning when OpenRouter reports that the route supports reasoning. Routes outside the table do not get proactive compaction or the header context counter until they are added to the catalog.

### Compaction

OpenRouter accepts OpenAI's `context_management` field but does not compact server-side (a request with a 500-token threshold and a 4,000-token prompt returns no compaction item). OpenRouter catalog entries therefore set `supports_compaction = false`, and Roder compacts client-side at 90% of the window.

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
model = "moonshotai/kimi-k3"

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
  "model": "moonshotai/kimi-k3",
  "stream": true
}
```

Roder does not send OpenAI encrypted reasoning replay fields to OpenRouter by default. Live checks are opt-in:

```sh
RODER_OPENROUTER_LIVE=1 OPENROUTER_LIVE_MODEL=moonshotai/kimi-k3 cargo test -p roder-ext-openrouter --test live_openrouter -- --ignored
```

## Running Under tmux

Inside tmux, Roder restarts its pane once to enable extended key reporting. The restarted process receives the launching shell's environment explicitly (`tmux respawn-pane -e KEY=VALUE`), so an `OPENROUTER_API_KEY` exported only in that shell still reaches it. Older builds dropped it and reported `OpenRouter API key is missing`; if you see that on an old build, put the key in `config.toml` or set it with `tmux set-environment -g`.
