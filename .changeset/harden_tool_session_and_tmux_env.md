---
roder-tools: patch
roder-tui: patch
roder-api: patch
roder: patch
---

# Harden tool process groups and the tmux environment handoff

Follow-up to the TUI suspend fix.

- On unix, a `shell` command that times out or is cancelled, and an
  `exec_command` session that times out, now has its whole process group
  killed. Tool children run in their own session, so killing only the shell left
  everything it had started running. A command that finishes by itself leaves
  its background processes alone, unless they keep its output pipes open, in
  which case the command is still waiting and the timeout applies.
- Inside tmux, the launching environment is now handed to the respawned roder
  through a private `0600` file that a small `sh` wrapper sources and deletes,
  instead of `respawn-pane -e` arguments that any local user could read from the
  process list. Non-UTF-8 values are preserved, and a failed respawn no longer
  stops the TUI from starting.
- Removes an unused import from the OpenRouter catalog module.
