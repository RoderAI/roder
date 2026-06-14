# roder

`roder` is the the end-user CLI and TUI for [Roder](https://roder.sh).

## What It Does

It builds the `roder` binary, wires the first-party extensions together, and provides the command-line, TUI, app-server, package, auth, exec, workflow, and automation entrypoints users run locally.

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
