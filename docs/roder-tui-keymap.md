# Roder TUI Keymap

Press `?` in the TUI to open the keyboard help overlay. Press `?` again or `Esc` to close it.

The default keymap keeps mouse interactions reachable from the keyboard:

| Action | Default binding |
| --- | --- |
| Open palette | `Ctrl+K` |
| Cycle mode | `Shift+Tab` |
| Focus next region | `Tab` |
| Focus previous region | `Shift+Tab` |
| Scroll transcript | `PageDown`, `PageUp` |
| Expand tool call | `Enter` |
| Collapse tool call | `Enter` |
| Open URL | `Enter` |
| Open file reference | `Enter` |
| Fold message | `Enter` |
| Open context menu | `Shift+F10` |
| Copy selection | `Ctrl+Shift+C` |
| Paste to composer | `Ctrl+Shift+V` |
| Approve hunk | `Y` |
| Reject hunk | `N` |

Bindings can be overridden with `[tui.keymap.bindings]` entries in the user config. The supported action ids are `expand_tool_call`, `collapse_tool_call`, `open_url`, `open_file_ref`, `fold_message`, `open_context_menu`, `copy_selection`, `paste_to_composer`, `approve_hunk`, `reject_hunk`, `open_palette`, `cycle_mode`, `scroll_transcript`, `focus_next_region`, and `focus_previous_region`.
