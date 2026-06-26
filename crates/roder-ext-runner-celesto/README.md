# roder-ext-runner-celesto

`roder-ext-runner-celesto` is the Celesto AI runner provider for [Roder](https://roder.sh).

## What It Does

It lets Roder run a thread's coding tools inside a Celesto sandboxed "computer":
it creates (or reuses) a Celesto computer, executes shell commands, reads and
writes files, publishes preview ports, and manages the computer lifecycle over
the Celesto Computers REST API.

Set `CELESTO_API_KEY` (or `RODER_CELESTO_API_KEY`) to use the provider. Override
the API endpoint with `CELESTO_BASE_URL` when needed.

## How It Fits Into Roder

Roder is an agentic software development system with a Rust CLI/TUI, a JSON-RPC
app-server, SDKs, package resources, and first-party runtime extensions. This
package is released as part of that workspace so downstream users can depend on
the Celesto remote-runner provider directly.

## Links

- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```
