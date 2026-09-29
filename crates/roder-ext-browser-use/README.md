# roder-ext-browser-use

`roder-ext-browser-use` is the browser-use browser provider for [Roder](https://roder.sh).

## What It Does

It launches the open-source browser-use local MCP server (`uvx --from 'browser-use[cli]' browser-use --mcp`) over stdio and exposes its browser tools to the agent as policy-gated `browser_use_*` tools. The server gets only an allowlisted environment plus the OpenAI/Anthropic keys Roder already holds, page content is labeled untrusted, and the server and its browser are stopped when Roder exits.

Enable it with `roder --browser-use` or `[browser_use] enabled = true`. See `docs/roder-browser-use-provider.md` in the repository.

## How It Fits Into Roder

Roder is an agentic software development system with a Rust CLI/TUI, a JSON-RPC app-server, SDKs, package resources, and first-party runtime extensions. This package is released as part of that workspace so downstream users can depend on the same component boundaries that Roder itself uses.

## Links

- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```
