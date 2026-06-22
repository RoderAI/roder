---
roder-tui-util: minor
roder-tui: patch
---

# Extract std-only TUI helpers into roder-tui-util

Three dependency-free `app`-module helpers moved out of `roder-tui` into a new
`roder-tui-util` crate: `scroll_accel` (scroll acceleration state machine),
`turn_timer` (turn elapsed-time tracking), and `progress` (terminal
progress-line writer). They are `std`-only, so the new crate has no
dependencies and compiles in parallel with the rest of the TUI. No behavior
change; consumers import via `roder_tui_util::`.
