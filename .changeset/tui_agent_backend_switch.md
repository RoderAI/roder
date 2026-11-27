---
roder-tui: minor
roder: minor
---

# Switch agent backends from the Ctrl+P menu

The Ctrl+P menu has a new **Agent backend** entry for Roder, Codex, and
Antigravity. Choosing a different backend restarts Roder with
`--backend <id>`, keeping your other flags (such as `--yolo` and
`--config-dir`) and dropping the backend-specific `--model` and `resume`
selections.
