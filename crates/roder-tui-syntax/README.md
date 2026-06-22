# roder-tui-syntax

Lightweight, dependency-free syntax highlighting used by the Roder TUI.

Provides a small tokenizer and `ratatui` span helpers (`highlight_code`,
`padded_highlighted_code`, `language_for_path`) for rendering code in the
transcript and diff views. Extracted from `roder-tui` so it compiles in
parallel with the rest of the TUI crate.
