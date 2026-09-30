# roder-ext-openai-responses

`roder-ext-openai-responses` is the OpenAI Responses provider for [Roder](https://roder.sh).

## What It Does

It supports OpenAI Responses-style inference, reasoning summaries, tool calls, and streaming events.

For the OpenAI API provider, a registered `computer` executor is advertised as
the native `{"type":"computer"}` tool. Ordered `computer_call` actions and
matching `computer_call_output` screenshots survive HTTP replay and WebSocket
continuation. See [native computer use](../../docs/native-computer-use.md) for
the executor and eval. The signed-in Codex endpoint does not support this tool.

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
