## 0.1.1 (2026-06-24)

### Features

#### Manage external web-search providers from the TUI

Extend the "Web search provider" settings submenu to list the external provider
router options (firecrawl, tavily, perplexity, parallel, synthetic) alongside
the hosted modes, showing each provider's enabled and API-key-configured status
and the active selection. Selecting an external provider persists
`[web_search] mode = "external"`, the chosen `provider`, and enables that
provider's sub-section in user config (applies on restart).

The same hosted modes and external providers are also selectable from the
`Ctrl+P` command palette, and both the menu and palette fall back to reading
providers directly from user config when app-server settings are unavailable.

`settings/get` and `settings/set_web_search` now report the external-router
snapshot via `WebSearchSettings { external_enabled, external_provider, providers }`
and accept an optional `external_provider` selection. New `roder-config`
helpers (`save_web_search_external_provider`, `WebSearchConfig::provider_configured`,
`WebSearchConfig::router_snapshot`, `web_search_router_snapshot`) back the
persistence, status checks, and config-only fallback.

Synthetic web search now auto-configures from the synthetic inference provider:
because both share `SYNTHETIC_API_KEY`, pasting the synthetic provider key
(`providers/configure`) makes the synthetic search provider report as
`key configured`, enables its `[web_search.synthetic]` sub-section, and lets it
resolve the borrowed key at runtime — no separate web-search key entry needed.

## 0.1.0 (2026-06-22)

### Features

#### Synthetic web search provider

Add a first-party web search provider backed by Synthetic's `/v2/search` HTTP API. The crate exposes a `SyntheticSearchExtension` whose manifest declares the `synthetic-search` tool provider, the `network.api.synthetic.new` capability, and the `SYNTHETIC_API_KEY` secret. It produces a canonical `web_search` tool when installed through `roder-ext-web-search` and a namespaced `synthetic_search` tool when `namespaced_tools = true`. Configuration is sourced from `[web_search.synthetic]`, `SYNTHETIC_BASE_URL`, and `RODER_SYNTHETIC_BASE_URL`, never from OpenAI/Anthropic variables.

#### Synthetic search offline normalization

Normalize Synthetic `/v2/search` responses into the canonical `WebSearchResponse` shape used by every other Roder search provider. Map `results[].{url,title,text,published}` into `WebSearchResult` (with extra fields preserved under `metadata`), preserve the raw response for debugging when configured, and redact API keys from any surfaced error messages.
