# Provider-Native Tool Search

Roder supports a canonical tool-search hint that lets supported providers defer loading tool definitions until the model searches for them. This keeps the runtime policy boundary unchanged: selected tools still execute through Roder's normal tool registry, permission checks, hooks, policy modes, transcript recording, and app-server events.

## Configuration

Provider-native tool search is off by default. Enable it globally with:

```toml
[tool_search]
mode = "provider_native" # explicit | auto | provider_native
max_catalog_items = 200
include_mcp = true
include_skills = true
fallback_to_explicit_tools = true
provider_variant = "regex" # default | regex | bm25; Anthropic only
```

The environment variable `RODER_TOOL_SEARCH_MODE=provider_native` can override the global mode for local experiments.

## OpenAI Responses

For supported OpenAI GPT models, Roder maps provider-native mode to Responses tools by:

- adding `defer_loading: true` to Roder function tools; and
- adding `{ "type": "tool_search" }` to the Responses `tools` array.

Unsupported models keep the current explicit tool-list request shape.

## Anthropic Messages

For supported direct Anthropic Claude models, Roder maps provider-native mode by:

- adding either `tool_search_tool_regex_20251119` or `tool_search_tool_bm25_20251119`; and
- adding `defer_loading: true` to deferred Roder tool definitions.

The tool-search helper itself is never deferred. Unsupported models keep the current explicit Anthropic `tools` request shape.

## Streamed Search Events

Provider-side searches surface through the existing canonical hosted
tool-call lifecycle, so timelines, transcripts, and app-server events stay
vendor-neutral:

- OpenAI Responses `tool_search_call` output items map to
  `HostedToolCallStarted` / `HostedToolCallCompleted` events named
  `tool_search`; the search query and any searched tool selections are
  preserved in the completion arguments (`query`, `selected_tools`).
- Anthropic `server_tool_use` blocks named `tool_search` map to the same
  hosted tool-call events with the accumulated search input as arguments.
  `tool_search_tool_result` and `tool_reference` blocks remain available in
  provider metadata, and discovered `tool_use` blocks execute through the
  normal Roder tool-call lifecycle.

## Offline Evals

`evals/tool_search/` holds the offline fixture matrix (explicit vs
provider-native comparisons, large catalog budgets, unsupported-model
fallback and fail-closed cases, malformed search results, unknown selected
tool ids, denied permissions, and catalog redaction). Run it with:

```sh
cargo test -p roder-evals tool_search --lib
```

## Safety Boundary

Provider tool search only changes the provider request body. It does not grant tool execution. Roder remains authoritative for:

- permission prompts and policy modes;
- hook execution;
- tool allowlists and path scopes;
- transcript and audit records; and
- app-server/ACP-visible tool-call events.

Live provider validation is opt-in and should only run when explicitly requested:

```sh
RODER_OPENAI_TOOL_SEARCH_LIVE=1 cargo test -p roder-ext-openai-responses live_openai_tool_search -- --ignored
RODER_ANTHROPIC_TOOL_SEARCH_LIVE=1 cargo test -p roder-ext-anthropic live_anthropic_tool_search -- --ignored
```
