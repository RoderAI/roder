## 0.1.0 (2026-06-22)

### Features

#### Synthetic web search provider

Add a first-party web search provider backed by Synthetic's `/v2/search` HTTP API. The crate exposes a `SyntheticSearchExtension` whose manifest declares the `synthetic-search` tool provider, the `network.api.synthetic.new` capability, and the `SYNTHETIC_API_KEY` secret. It produces a canonical `web_search` tool when installed through `roder-ext-web-search` and a namespaced `synthetic_search` tool when `namespaced_tools = true`. Configuration is sourced from `[web_search.synthetic]`, `SYNTHETIC_BASE_URL`, and `RODER_SYNTHETIC_BASE_URL`, never from OpenAI/Anthropic variables.

#### Synthetic search offline normalization

Normalize Synthetic `/v2/search` responses into the canonical `WebSearchResponse` shape used by every other Roder search provider. Map `results[].{url,title,text,published}` into `WebSearchResult` (with extra fields preserved under `metadata`), preserve the raw response for debugging when configured, and redact API keys from any surfaced error messages.
