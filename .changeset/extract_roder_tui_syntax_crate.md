---
roder-tui-syntax: minor
roder-tui: patch
---

# Extract syntax highlighting into roder-tui-syntax

The TUI's syntax highlighter (tokenizer + `ratatui` span helpers) moved out of
`roder-tui` into a new dependency-light `roder-tui-syntax` crate. This is the
first step of the build-time crate-splitting work: the highlighter now compiles
in parallel with the rest of `roder-tui` instead of as part of its single
translation unit. No behavior change; `roder-tui` re-imports the same API via
`roder_tui_syntax::`.
