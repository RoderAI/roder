---
roder-tui: patch
---

# Keep chat prompts readable on light terminal themes

Inherit the terminal background when its light/dark appearance is unknown,
instead of forcing a dark prompt surface. The bundled light theme now sets a
matching light prompt background.
