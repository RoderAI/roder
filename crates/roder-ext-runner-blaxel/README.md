# roder-ext-runner-blaxel

`roder-ext-runner-blaxel` is the Blaxel runner provider for [Roder](https://roder.sh).

## What It Does

It runs a Roder thread's coding tools inside a [Blaxel sandbox](https://docs.blaxel.ai/Overview)
— an instant-launching micro VM that scales to standby when idle and resumes in
milliseconds with its filesystem and processes intact. The provider drives the
Blaxel control-plane (`/sandboxes`) and per-sandbox REST APIs (process,
filesystem, preview) and supports the full runner lifecycle: pause, resume,
detach, and rejoin.

Credentials come from `BLAXEL_API_KEY` (or `BL_API_KEY`) plus `BL_WORKSPACE` and
are never written to session state. See
[`docs/roder-blaxel-runner.md`](https://github.com/RoderAI/roder/blob/master/docs/roder-blaxel-runner.md)
for setup, configuration, and the lifecycle model.

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
