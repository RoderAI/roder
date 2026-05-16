# Roder Web Search

Roder uses Codex/OpenAI hosted web search by default. Hosted search is sent to the Responses API as a provider-native `web_search` tool, so it is not listed as a local `tools/list` executor.

Use `mode = "live"` to allow live internet access, or `mode = "disabled"` to turn hosted web search off:

```toml
[web_search]
mode = "live" # default is "codex", which uses cached hosted search
```

Set `RODER_WEB_SEARCH_MODE=live`, `codex`, `external`, or `disabled` to override this from the environment.

The TUI command palette (`Ctrl+P`) exposes hosted web search choices under Settings: Codex cached, Codex live, and Disabled. External provider-router mode still requires config because those local tools are installed when the registry is built.

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

Supported provider values are `firecrawl`, `tavily`, `perplexity`, and `parallel`. Explicit config values win over environment values; environment variables fill missing config values.

## Environment

- `FIRECRAWL_API_KEY`, `FIRECRAWL_BASE_URL`
- `TAVILY_API_KEY`, `TAVILY_PROJECT`, `TAVILY_BASE_URL`
- `PERPLEXITY_API_KEY`, `PERPLEXITY_BASE_URL`
- `PARALLEL_API_KEY`, `PARALLEL_BASE_URL`

## Provider Fit

- Firecrawl: web search with optional markdown page extraction through `include_content`.
- Tavily: fast structured search with answer and raw-content options.
- Perplexity: ranked search results; Sonar answer mode is intentionally deferred until bounded normalization is stable.
- Parallel: objective-oriented search with LLM-optimized excerpts.

## Local And Live Tests

Local tests do not require credentials:

```sh
cargo test -p roder-ext-firecrawl-search
cargo test -p roder-ext-tavily-search
cargo test -p roder-ext-perplexity-search
cargo test -p roder-ext-parallel-search
cargo test -p roder-extension-host -p roder-app-server web_search
```

Live smoke tests are opt in:

```sh
RODER_LIVE_WEB_SEARCH=1 FIRECRAWL_API_KEY="$FIRECRAWL_API_KEY" cargo test -p roder-ext-firecrawl-search --test firecrawl_search -- --ignored
RODER_LIVE_WEB_SEARCH=1 TAVILY_API_KEY="$TAVILY_API_KEY" cargo test -p roder-ext-tavily-search --test live -- --ignored
RODER_LIVE_WEB_SEARCH=1 PERPLEXITY_API_KEY="$PERPLEXITY_API_KEY" cargo test -p roder-ext-perplexity-search --test live -- --ignored
RODER_LIVE_WEB_SEARCH=1 PARALLEL_API_KEY="$PARALLEL_API_KEY" cargo test -p roder-ext-parallel-search --test live -- --ignored
```

Tool specs, extension manifests, errors, and app-server discovery responses must not expose API keys or auth headers.
