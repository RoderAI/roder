# roder-ext-kimi-code

`roder-ext-kimi-code` is the Kimi Code (Moonshot AI) provider bridge for [Roder](https://roder.sh).

## What It Does

It connects Roder to Kimi Code subscription inference (device OAuth against auth.kimi.com + OpenAI-compatible chat completions at api.moonshot.ai/v1).

## How It Fits Into Roder

Roder is an agentic software development system with a Rust CLI/TUI, a JSON-RPC app-server, SDKs, package resources, and first-party runtime extensions. This package is released as part of that workspace so downstream users can depend on it when building custom distributions.

## Links

- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder
- Kimi Code: https://kimi.com/code (or auth.kimi.com for the subscription flow)

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```
