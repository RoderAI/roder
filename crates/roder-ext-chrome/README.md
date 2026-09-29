# roder-ext-chrome

`roder-ext-chrome` is the Chrome browser extension bridge for [Roder](https://roder.sh).

## What It Does

It lets Roder inspect and control the user's live Chrome session for browser-aware tasks.

It also holds Roder's own direct CDP toolset (`roder_ext_chrome::direct`):
`look`, `screenshot`, `click`, `hover`, `drag`, `type`, `key`, `scroll`,
`select`, `navigate` and `wait` on one tab, with real DevTools input, reached
through a browser's DevTools endpoint and the tab's target id (or a page
websocket). The owner of the tab supplies its rules through a `DirectGuard`
(allowed origins, a gate before irreversible actions, typed secrets to keep
out of reads and screenshots, what an access block looks like), and
`direct_tools` turns the set into model-facing tools bound to a tab by a
`DirectBinding`. The `chrome_*` tools use it for Roder Desktop's integrated
browser when no extension is connected, and `roder-ext-jev` binds it to its
session's tab as the `jev_tab_*` tools and its automatic fallback.

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
