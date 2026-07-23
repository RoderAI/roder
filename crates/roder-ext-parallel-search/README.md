# roder-ext-parallel-search

Parallel.ai web search + URL extract provider for [Roder](https://roder.sh).

## What It Does

Contributes two tools:

- `parallel_search` — `POST /v1/search` with an objective (and optional keyword
  `search_queries`), returning ranked URL + excerpt results.
- `parallel_extract` — `POST /v1/extract` for one or more public URLs, returning
  focused markdown excerpts and optional full page content.

When Parallel is selected as the external `web_search` provider, the canonical
`web_search` tool also routes through Parallel search. The runtime also appends
a short **Parallel Web Access** developer-instruction block so the model knows
to search first, then `parallel_extract` deeper URLs.

## Configuration

In `~/.roder/config.toml`:

```toml
[web_search]
enabled = true
mode = "external"
provider = "parallel"
# namespaced_tools = true  # also expose other providers' *_search tools
# Note: selecting provider = "parallel" always installs parallel_extract.

[web_search.parallel]
enabled = true
api_key = "..." # or set PARALLEL_API_KEY
# mode = "advanced" # turbo | basic | advanced (default advanced)
```

Optional env:

- `PARALLEL_API_KEY`
- `PARALLEL_BASE_URL` (default `https://api.parallel.ai`)

## Search request mapping

| Roder field | Parallel field |
| --- | --- |
| `query` | `objective` (+ derived `search_queries` when omitted) |
| `search_queries` | top-level `search_queries` |
| `max_results` | `advanced_settings.max_results` |
| `include_domains` / `exclude_domains` | `advanced_settings.source_policy.*` |
| `country` | `advanced_settings.location` |
| provider `mode` | top-level `mode` (`turbo` / `basic` / `advanced`) |

## Extract tool arguments

| Argument | Notes |
| --- | --- |
| `urls` / `url` | Required. Up to 20 http(s) URLs |
| `objective` / `query` | Focus excerpts on a goal |
| `search_queries` | Optional keyword focus |
| `full_content` / `include_content` | Enable full markdown content |
| `max_chars_total` | Cap total excerpt characters |
| `max_chars_per_result` | Cap per-URL excerpt/full content |
| `session_id` | Link extract to a prior search session |

Auth header: `x-api-key`.

## Live smoke

```sh
export PARALLEL_API_KEY=...
export RODER_LIVE_WEB_SEARCH=1
cargo test -p roder-ext-parallel-search --test live -- --ignored
```

## Links

- Parallel Search quickstart: https://docs.parallel.ai/search/search-quickstart
- Parallel Extract quickstart: https://docs.parallel.ai/extract/extract-quickstart
- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```
