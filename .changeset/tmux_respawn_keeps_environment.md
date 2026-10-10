---
roder-tui: patch
roder: patch
---

# Keep the launching environment when roder respawns itself inside tmux

Inside tmux, roder restarts its pane once to enable extended key reporting.
`tmux respawn-pane` starts the new process with the tmux server's environment,
not the environment roder was launched with, so anything exported only in the
launching shell (provider API keys such as `OPENROUTER_API_KEY`, direnv or mise
values) was silently dropped and the relaunched TUI reported its credentials as
missing. The respawn now passes the full environment through explicitly.
