# Roder Kimi Code Provider

Roder exposes Kimi Code (Moonshot AI) models via the dedicated `kimi-code` provider (and short aliases `kimi`, `moonshot`).

Model labels use `<provider>/<model>`, for example:
```text
kimi-code/kimi-for-coding
```

This is the direct Kimi Code subscription route (distinct from the `opencode-go/kimi-k2.6` path which goes through OpenCode's Go surface).

## Authentication

Kimi Code supports two auth paths with **different API surfaces**:

1. **Device OAuth (Kimi Code subscription)** — `https://api.kimi.com/coding/v1` with Kimi device headers and `kimi-code-cli` User-Agent (matches the official Kimi Code CLI).
2. **Direct API key (Moonshot Open Platform)** — `https://api.moonshot.ai/v1` (or `api.moonshot.cn/v1` for CN keys).

### Device OAuth

```sh
roder auth login kimi-code
roder auth status kimi-code
roder auth logout kimi-code
```

The TUI provider menu (`Ctrl+P`) also exposes login/logout via `auth/kimi-code/*` JSON-RPC methods.

Tokens are stored under `~/.roder/auth/kimi-code.json`. A stable device id is kept at `~/.roder/auth/kimi-code-device-id` for Kimi device headers.

Managed API base URL override (optional):
```sh
export KIMI_CODE_BASE_URL="https://api.kimi.com/coding/v1"
# or
export RODER_KIMI_CODE_BASE_URL="https://api.kimi.com/coding/v1"
```

OAuth host override (optional):
```sh
export KIMI_CODE_OAUTH_HOST="https://auth.kimi.com"
```

### API key fallback (Open Platform)

Recommended env vars:
```sh
export KIMI_CODE_API_KEY="..."
# or
export RODER_KIMI_CODE_API_KEY="..."
```

You can also configure via the TUI provider menu or in `~/.roder/config.toml`:
```toml
[providers.kimi-code]
api_key = "..."
# optional override for Open Platform keys
base_url = "https://api.moonshot.ai/v1"
```

API keys take precedence over OAuth tokens when both are configured.

## Models

Default OAuth model:
- `kimi-for-coding` ("K2.7 Code") — served from `GET /models` on the managed API

Additional models may appear as Kimi expands the managed catalog.

## Relationship to OpenCode

- `opencode-go/kimi-k2.6` routes through OpenCode's Zen/Go service (different auth surface and headers).
- `kimi-code/kimi-for-coding` talks the Kimi Code managed API directly with your subscription OAuth token.

Users with both subscriptions can choose the route that best matches their quota / features / latency.

## Capabilities

The provider re-uses the OpenAI Chat Completions streaming helper:
- Streaming responses
- Tool calling (including parallel)
- Structured output

## Troubleshooting

- "Kimi Code auth is missing" → run `roder auth login kimi-code` or set `KIMI_CODE_API_KEY` / `RODER_KIMI_CODE_API_KEY`.
- 401 on OAuth → re-run `roder auth login kimi-code`; OAuth tokens are not valid on `api.moonshot.ai/v1`.
- 403 "only available for Coding Agents" → ensure device headers are sent (built into the provider); do not point OAuth at the Open Platform URL.

See also:
- `docs/roder-opencode-providers.md` (for the alternative Go path)