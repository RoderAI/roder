---
roder-tui-diff: minor
roder-tui: patch
---

# Extract diff viewer into roder-tui-diff

The TUI diff subsystem (line-diff engine, hunk navigation/keymap, and the
unified/side-by-side `ratatui` renderer, plus the `DiffViewerState` model) moved
out of `roder-tui` into a new `roder-tui-diff` crate so it compiles in parallel
with the rest of the TUI. It depends only on `ratatui`, `roder-api`, and
`roder-tui-syntax`. No behavior change; the (currently unwired) `diff_ui`
integration now imports the API via `roder_tui_diff::`. `similar` is dropped
from `roder-tui`'s direct dependencies since only the diff engine used it.
