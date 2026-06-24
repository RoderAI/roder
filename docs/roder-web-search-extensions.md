# Roder Web Search

Roder uses OpenAI hosted web search by default. Hosted search is sent to the Responses API as a provider-native `web_search` tool, so it is not listed as a local `tools/list` executor.

Use `mode = "live"` to allow live internet access, or `mode = "disabled"` to turn hosted web search off:

```toml
[web_search]
mode = "live" # default is "cached", which uses cached hosted search
```

Set `RODER_WEB_SEARCH_MODE=live`, `cached`, `external`, or `disabled` to override this from the environment.

The TUI command palette (`Ctrl+P`) exposes hosted web search choices under Settings: Cached hosted, Live hosted, and Disabled. External provider-router mode still requires config because those local tools are installed when the registry is built.

## Managing Providers In The TUI

The settings menu item "Web search provider" opens a submenu that lists both
surfaces in one place:

- **Hosted (OpenAI/Codex)**: Cached hosted, Live hosted, Disabled. Selecting one
  applies immediately and persists the hosted `mode`.
- **External providers**: firecrawl, tavily, perplexity, parallel, synthetic.
  Each row shows whether the provider sub-section is `enabled` and whether an API
  key is resolvable (`key configured` / `no API key`), and marks the active
  selection. Selecting one writes `[web_search] mode = "external"`, the chosen
  `provider`, and enables that provider's sub-section in `~/.roder/config.toml`.
  External-router changes take effect on the next start because the provider
  tools are installed during extension-host registration.

The same hosted modes and external providers are also exposed in the `Ctrl+P`
command palette under Settings (search for "web search"), each labelled with its
key-configured status. When the app-server settings cannot be fetched, the menu
and palette fall back to reading the providers directly from
`~/.roder/config.toml`, so the list is always available.

### Synthetic auto-setup

Synthetic web search shares `SYNTHETIC_API_KEY` with the Synthetic Chat
Completions provider. When you configure the `synthetic` inference provider and
paste its key (stored under `[providers.synthetic]`), the synthetic search
provider auto-configures: it reports as `key configured` in the menu, its
`[web_search.synthetic]` sub-section is enabled, and it borrows that key at
runtime — you do not need to re-enter a key under `[web_search.synthetic]`.
Selecting Synthetic in the submenu then just works on the next start.

## External Provider Router

Roder can also install web search through native extension crates. In this mode the canonical local tool is `web_search`; provider-specific tools are only exposed when `namespaced_tools = true`.

## Config

Add one provider to `~/.roder/config.toml`:

```toml
[web_search]
mode = "external"
provider = "tavily"
namespaced_tools = false
max_results = 8
timeout_seconds = 20

[web_search.tavily]
enabled = true
api_key_env = "TAVILY_API_KEY"
project_env = "TAVILY_PROJECT"
base_url = "https://api.tavily.com"
search_depth = "basic"
```

Supported provider values are `firecrawl`, `tavily`, `perplexity`, `parallel`, and `synthetic`. Explicit config values win over environment values; environment variables fill missing config values.

## Environment

- `FIRECRAWL_API_KEY`, `FIRECRAWL_BASE_URL`
- `TAVILY_API_KEY`, `TAVILY_PROJECT`, `TAVILY_BASE_URL`
- `PERPLEXITY_API_KEY`, `PERPLEXITY_BASE_URL`
- `PARALLEL_API_KEY`, `PARALLEL_BASE_URL`
- `SYNTHETIC_API_KEY`, `SYNTHETIC_BASE_URL`, `RODER_SYNTHETIC_BASE_URL`

## Provider Fit

- Firecrawl: web search with optional markdown page extraction through `include_content`.
- Tavily: fast structured search with answer and raw-content options.
- Perplexity: ranked search results; Sonar answer mode is intentionally deferred until bounded normalization is stable.
- Parallel: objective-oriented search with LLM-optimized excerpts.
- Synthetic: Synthetic's first-party `/v2/search` HTTP API; backed by the same `SYNTHETIC_API_KEY` already used for Synthetic Chat Completions, so existing Synthetic subscriptions work without an extra account.

## Local And Live Tests

Local tests do not require credentials:

```sh
cargo test -p roder-ext-firecrawl-search
cargo test -p roder-ext-tavily-search
cargo test -p roder-ext-perplexity-search
cargo test -p roder-ext-parallel-search
cargo test -p roder-ext-synthetic-search
cargo test -p roder-extension-host -p roder-app-server web_search
```

Live smoke tests are opt in:

```sh
RODER_LIVE_WEB_SEARCH=1 FIRECRAWL_API_KEY="$FIRECRAWL_API_KEY" cargo test -p roder-ext-firecrawl-search --test firecrawl_search -- --ignored
RODER_LIVE_WEB_SEARCH=1 TAVILY_API_KEY="$TAVILY_API_KEY" cargo test -p roder-ext-tavily-search --test live -- --ignored
RODER_LIVE_WEB_SEARCH=1 PERPLEXITY_API_KEY="$PERPLEXITY_API_KEY" cargo test -p roder-ext-perplexity-search --test live -- --ignored
RODER_LIVE_WEB_SEARCH=1 PARALLEL_API_KEY="$PARALLEL_API_KEY" cargo test -p roder-ext-parallel-search --test live -- --ignored
RODER_LIVE_WEB_SEARCH=1 SYNTHETIC_API_KEY="$SYNTHETIC_API_KEY" cargo test -p roder-ext-synthetic-search --test live -- --ignored
```

Tool specs, extension manifests, errors, and app-server discovery responses must not expose API keys or auth headers.
