# roder-app-server

`roder-app-server` is the local app-server runtime for [Roder](https://roder.sh).

## What It Does

It exposes the Roder runtime over JSON-RPC and WebSocket transports for the TUI, SDKs, ACP adapter, remote nodes, and integration tests.

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
