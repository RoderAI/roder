# roder-tui

`roder-tui` is the terminal user interface for [Roder](https://roder.sh).

## What It Does

It renders the interactive Roder TUI, including transcript timelines, tools, palettes, forks, knowledge, remote nodes, composer state, and dialogs.

## Transcript image previews

Local image attachments and successful `view_image`, `media_generate_image`, and
`media_attach` results can render pixel previews in the transcript. PNG, JPEG,
and WebP are decoded on a background worker, with a 32 MiB encoded-input limit,
a 64 MiB raster limit, and a maximum preview of 48 columns by 20 rows.
Image pixels never become message source or clipboard content.

Kitty and Ghostty are recognized through their terminal environment; Foot/Sixel
also requires measured terminal cell dimensions. Unknown terminals retain a
file label instead of emitting unsupported escape sequences. Set
`RODER_IMAGE_PROTOCOL=none` to disable pixels, or `kitty` / `sixel` to explicitly
select a supported protocol. Under tmux, previews default to labels: opt in only
when the outer terminal supports that protocol and tmux passthrough is enabled
(`set -g allow-passthrough on`). Sixel still requires measurable cell dimensions.

Previews reserve transcript rows, clip at both viewport edges, re-encode after
width changes, and never paint into the composer. Kitty uses virtual placements
and deletes only Roder-owned image data when a preview is replaced or evicted.
All pixel previews are suspended while a popup is open; owned Kitty placements
and visible Sixel cells are cleared before the popup is drawn, and protocols are
rebuilt when previews resume.
Remote URLs and arbitrary paths in prose are not fetched or decoded.
`view_image` arguments are resolved only for a known local thread, relative to
that thread's authoritative workspace rather than the UI process directory.
Remote and unknown-origin timelines do not reinterpret runner paths as host
files; explicit host media artifacts must carry absolute storage paths.

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
