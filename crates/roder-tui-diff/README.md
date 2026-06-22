# roder-tui-diff

Diff computation, hunk navigation, and `ratatui` diff-viewer rendering for the
Roder TUI.

Holds the `DiffViewerState` model, the line-level diff engine (`compute`), the
keymap/hunk-resolution logic (`keys`), and the unified/side-by-side renderer
(`render`). Extracted from `roder-tui` so it builds in parallel with the rest of
the TUI crate. Depends only on `ratatui`, `roder-api`, and `roder-tui-syntax`.
