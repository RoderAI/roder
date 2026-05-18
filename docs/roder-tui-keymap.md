# Roder TUI Keymap

The TUI keeps keyboard actions aligned with mouse interactions so clickable regions remain reachable without a mouse.

## Defaults

| Action | Default keys |
| --- | --- |
| Open palette | `ctrl+p` |
| Cycle mode | `shift+tab` |
| Focus next region | `tab` |
| Focus previous region | `shift+tab` |
| Expand or collapse a tool call | `enter` |
| Open a focused URL or file reference | `enter`, `o` |
| Fold or unfold a long message | `enter` |
| Copy active selection | `c` |
| Paste transcript selection to composer | `p` |
| Approve / reject focused diff hunk | `a`, `r` |
| Scroll focused surface | mouse wheel |
| Fast scroll focused surface | `ctrl` + mouse wheel |

## Configuration

Key bindings can be overridden in the user config under `[tui.keymap]`.

```toml
[tui.keymap]
"palette/open" = ["ctrl+k"]
"selection/copy" = ["y"]
```

Action ids are defined by `roder_tui::keymap::Action`.
