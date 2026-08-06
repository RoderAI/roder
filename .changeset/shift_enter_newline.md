---
roder-tui: patch
roder: patch
---

# Fix Shift+Enter newline in the composer

Make Shift+Enter insert a newline instead of submitting. Inside tmux, enable
CSI-u extended keys (respawning the pane once when needed) so the terminal can
distinguish Shift+Enter from Enter. Also accept Ctrl+J and Alt+Enter as newline
fallbacks when the terminal cannot report modifiers.
