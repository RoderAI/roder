# Roder Kimi Code Provider

Roder exposes Kimi Code (Moonshot AI) models via the dedicated `kimi-code` provider (and short aliases `kimi`, `moonshot`).

Model labels use `<provider>/<model>`, for example:
```text
kimi-code/kimi-k2.6
```

This is the *direct* Moonshot route using your Kimi Code subscription/pack (distinct from the `opencode-go/kimi-k2.6` path which goes through OpenCode's Go surface).

## Authentication

Kimi Code supports two auth paths:

1. **Device OAuth (recommended for Kimi Code subscriptions)** — matches the official Kimi Code CLI flow against `auth.kimi.com`.
2. **Direct API key** — long-lived keys usable at `api.moonshot.ai/v1`.

### Device OAuth

```sh
roder auth login kimi-code
roder auth status kimi-code
roder auth logout kimi-code
```

The TUI provider menu (`Ctrl+P`) also exposes login/logout via `auth/kimi-code/*` JSON-RPC methods.

Tokens are stored under `~/.roder/auth/kimi-code.json`. A stable device id is kept at `~/.roder/auth/kimi-code-device-id` for Kimi device headers.

OAuth host override (optional):
```sh
export KIMI_CODE_OAUTH_HOST="https://auth.kimi.com"
# or
export KIMI_OAUTH_HOST="https://auth.kimi.com"
```

### API key fallback

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
# optional
base_url = "https://api.moonshot.ai/v1"
```

Base URL can be overridden with `RODER_KIMI_CODE_BASE_URL` or `KIMI_CODE_BASE_URL`.

API keys take precedence over OAuth tokens when both are configured.

## Models

Prominent model currently exposed under the provider (via the static catalog; dynamic discovery can be added later):
- `kimi-k2.6` ("Kimi K2.6")

Additional K2 variants may be added as they stabilize. The catalog entry lives in `roder-api`.

## Relationship to OpenCode

- `opencode-go/kimi-k2.6` routes through OpenCode's Zen/Go service (different auth surface and headers).
- `kimi-code/kimi-k2.6` talks the Moonshot API directly with your Kimi Code credentials.

Users with both subscriptions can choose the route that best matches their quota / features / latency.

## Capabilities

The provider re-uses the OpenAI Chat Completions streaming helper:
- Streaming responses
- Tool calling (including parallel)
- Structured output
- Standard context windows for the K2 models

Image input and certain advanced features are currently disabled (matching the initial catalog entry).

## Troubleshooting

- "Kimi Code auth is missing" → run `roder auth login kimi-code` or set `KIMI_CODE_API_KEY` / `RODER_KIMI_CODE_API_KEY`.
- 401/403 → verify your subscription or API key has access to the Moonshot `v1` surface.
- Models not appearing → ensure the `roder-ext-kimi-code` extension is loaded (it is by default in full distributions).

See also:
- `docs/roder-opencode-providers.md` (for the alternative Go path)
- Roadmap item `roadmap/95-kimi-code-provider.md` for implementation status