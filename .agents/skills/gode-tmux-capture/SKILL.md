---
name: gode-tmux-capture
description: Use when an agent needs to run the local gode TUI in tmux, send text or keys into the session, inspect panes, and capture pane output as timestamped .txt and .png artifacts.
---

# Gode Tmux Capture

Use this skill for interactive `gode` runs that need durable evidence. Prefer the bundled helper script over hand-written `tmux` commands.

## Quick Start

From the repo root:

```sh
.agents/skills/gode-tmux-capture/scripts/gode_tmux.sh start
.agents/skills/gode-tmux-capture/scripts/gode_tmux.sh send --enter "summarize this repo"
.agents/skills/gode-tmux-capture/scripts/gode_tmux.sh capture
```

The default session is `gode`, the default command is `go run ./cmd/gode`, and captures go to `.gode/tmux-captures/`.

## Common Commands

- Start `gode` in a detached tmux session:
  ```sh
  .agents/skills/gode-tmux-capture/scripts/gode_tmux.sh start -s gode -- go run ./cmd/gode
  ```
- Start with an alternate command:
  ```sh
  .agents/skills/gode-tmux-capture/scripts/gode_tmux.sh start -s gode-dev -- make run
  ```
- Send literal text without pressing Enter:
  ```sh
  .agents/skills/gode-tmux-capture/scripts/gode_tmux.sh send -s gode "draft a plan"
  ```
- Send literal text and press Enter:
  ```sh
  .agents/skills/gode-tmux-capture/scripts/gode_tmux.sh send -s gode --enter "draft a plan"
  ```
- Send tmux key names:
  ```sh
  .agents/skills/gode-tmux-capture/scripts/gode_tmux.sh keys -s gode Enter C-c
  ```
- Save pane output as both text and PNG:
  ```sh
  .agents/skills/gode-tmux-capture/scripts/gode_tmux.sh capture -s gode -o .gode/tmux-captures
  ```
- Attach manually when visual inspection is useful:
  ```sh
  .agents/skills/gode-tmux-capture/scripts/gode_tmux.sh attach -s gode
  ```
- Stop the session:
  ```sh
  .agents/skills/gode-tmux-capture/scripts/gode_tmux.sh stop -s gode
  ```

## Workflow

1. Use a named tmux session per task, usually `gode` or `gode-<short-task>`.
2. Start from the repo root unless the user explicitly asks for another working directory.
3. Capture before sending disruptive keys like `C-c`, after every meaningful interaction, and before reporting results.
4. Report the saved `.txt` and `.png` paths in the final answer when captures are part of the requested evidence.

## Notes

- The helper targets pane `0.0` in the named session.
- PNG capture uses macOS `qlmanage` when available, or a Python Pillow renderer if Pillow is installed.
- If PNG rendering is unavailable, keep the `.txt` capture and state that PNG rendering could not run in the current environment.
