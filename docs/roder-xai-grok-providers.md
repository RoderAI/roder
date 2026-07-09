# Roder xAI Grok Providers

Roder exposes Grok models through two provider ids:

- `xai`: direct xAI API-key auth with `XAI_API_KEY`.
- `supergrok`: SuperGrok subscription OAuth with `roder auth login supergrok`.

Both providers use `provider/model` labels such as `xai/grok-4.5`, `supergrok/grok-4.5`, and `supergrok/grok-build-0.1`. SuperGrok now dynamically queries the xAI `/models` (and `/v1/models`) endpoint (with caching + background refresh, using the SuperGrok OAuth token) to surface the latest models and basic capabilities from xAI. Static catalog entries (including grok-4.5 and grok-build-0.1) act as the curated fallback.

## Direct xAI API Key

Set an API key in the environment:

```sh
export XAI_API_KEY=...
roder --provider xai/grok-4.5
```

`RODER_XAI_API_KEY` is also accepted. The xAI base URL defaults to `https://api.x.ai/v1` and can be overridden with `RODER_XAI_BASE_URL` or `XAI_BASE_URL` for compatible test endpoints.

You can also store the API key in config:

```toml
[providers.xai]
api_key = "..."
```

## SuperGrok OAuth

SuperGrok uses the OAuth 2.0 Device Authorization Grant (device flow) for robust CLI / TUI login (with browser-assisted user code entry) and stores tokens under:

```text
$HOME/.roder/auth/supergrok.json
```

Commands:

```sh
roder auth login supergrok
roder auth status supergrok
roder auth logout supergrok
```

The TUI provider menu (`Ctrl+P`) starts the same browser OAuth flow when selecting SuperGrok while signed out.

OAuth aliases `grok-oauth`, `xai-oauth`, `x-ai-oauth`, and `xai-grok-oauth` normalize to `supergrok`.

## Models

Current visible Grok entries:

- `grok-4.5`: 500,000 token context, default high reasoning (low/medium/high). Primary default for xAI and SuperGrok.
- `grok-4.3`: 1,000,000 token context (legacy via static catalog).
- `grok-build-0.1`: Grok Build (xAI's agentic coding model); 500k context. Visible on SuperGrok; fetched live via `/models` when signed in.
- `grok-composer-2.5-fast`: 200,000 token context SuperGrok coding model (no image inputs).
- `grok-4.20-multi-agent-0309`: 2,000,000 token context, 1,800,000 token auto-compaction threshold.
- `grok-4.20-0309-reasoning`: 2,000,000 token context, 1,800,000 token auto-compaction threshold.
- `grok-4.20-0309-non-reasoning`: 2,000,000 token context, 1,800,000 token auto-compaction threshold.

Roder intentionally does not list retired Grok models such as `grok-3` or `grok-code-fast-1`.

## Request Shape

For xAI-compatible Responses requests, Roder:

- Sends `x-grok-conv-id` from the thread id.
- Sends body-level `prompt_cache_key` from the thread id.
- Omits `reasoning.encrypted_content` for xAI and SuperGrok.
- Sends `reasoning.effort` only for known reasoning-capable Grok models.

401 errors point users at API-key or SuperGrok login setup. 403 errors are reported as entitlement or quota checks and keep the provider's original error text.

## Live Tests

Normal tests do not require xAI network access. Live checks must be explicitly opted into with `RODER_XAI_LIVE=1` and valid xAI or SuperGrok credentials.
