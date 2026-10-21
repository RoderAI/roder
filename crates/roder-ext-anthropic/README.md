# roder-ext-anthropic

`roder-ext-anthropic` is the Anthropic inference provider for [Roder](https://roder.sh).

## What It Does

It connects Roder to Anthropic Messages-style streaming, tool calls, and SSE handling.

## Service tiers

Set `AgentInferenceRequest.runtime.service_tier` to `"standard_only"` to prevent
Priority Tier use, or `"auto"` to permit Anthropic's automatic tier selection.
When absent, the provider default remains in effect. Explicit values are sent
unchanged, so Anthropic rejects unsupported tiers rather than silently using a
different one. The reported tier is retained in `TokenUsage.service_tier` and
raw provider metadata; missing reports remain unknown.

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
