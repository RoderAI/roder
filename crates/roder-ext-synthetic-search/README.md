# roder-ext-synthetic-search

`roder-ext-synthetic-search` is the Synthetic web search provider for [Roder](https://roder.sh).

## What It Does

It contributes Synthetic-backed web search results to Roder. It wraps
Synthetic's documented `/v2/search` HTTP API (see
[dev.synthetic.new/docs/synthetic/search](https://dev.synthetic.new/docs/synthetic/search))
behind Roder's canonical `web_search` and namespaced `synthetic_search` tools.

## How It Fits Into Roder

Roder is an agentic software development system with a Rust CLI/TUI, a JSON-RPC app-server, SDKs, package resources, and first-party runtime extensions. This package is released as part of that workspace so downstream users can depend on it directly while still benefiting from first-party release tooling.

## Links

- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder
- Synthetic search docs: https://dev.synthetic.new/docs/synthetic/search

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```
