---
roder: patch
roder-tui: patch
---

# TUI renders markdown tables as aligned columns

Assistant markdown pipe tables now render as aligned grids with a header rule, instead of raw `|` rows. Tables that do not fit the transcript width fall back to labeled rows so wrapping does not smash the columns together.
