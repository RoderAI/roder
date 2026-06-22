# roder-tui-util

Small, dependency-free runtime helpers for the Roder TUI:

- `scroll_accel` — scroll acceleration state machine for mouse/key scrolling.
- `turn_timer` — elapsed-time tracking for an in-progress agent turn.
- `progress` — terminal progress-line writer (`ProgressReporter`).

Extracted from `roder-tui` (the `app` module) so these `std`-only utilities
compile in parallel with the rest of the TUI crate instead of as part of its
single large translation unit.
