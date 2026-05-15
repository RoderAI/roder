---
name: roder-tmux
description: Use when an agent needs to run the local roder TUI in tmux, send text or keys into the session, inspect panes, and capture pane output as timestamped .txt and .png artifacts.
---

# Roder Tmux Capture

Use this skill for interactive `roder` runs that need durable evidence. Prefer the bundled helper script over hand-written `tmux` commands.

## Quick Start

From the repo root:

```sh
.agents/skills/roder-tmux/scripts/roder_tmux.sh start
.agents/skills/roder-tmux/scripts/roder_tmux.sh send --enter "summarize this repo"
.agents/skills/roder-tmux/scripts/roder_tmux.sh capture
```

The default session is `roder`, the default command is `cargo run -p roder-cli --bin roder`, and captures go to `.roder/tmux-captures/`.

## Common Commands

- Start `roder` in a detached tmux session:
  ```sh
  .agents/skills/roder-tmux/scripts/roder_tmux.sh start -s roder -- cargo run -p roder-cli --bin roder
  ```
- Start with an alternate command:
  ```sh
  .agents/skills/roder-tmux/scripts/roder_tmux.sh start -s roder-dev -- make run
  ```
- Send literal text without pressing Enter:
  ```sh
  .agents/skills/roder-tmux/scripts/roder_tmux.sh send -s roder "draft a plan"
  ```
- Send literal text and press Enter:
  ```sh
  .agents/skills/roder-tmux/scripts/roder_tmux.sh send -s roder --enter "draft a plan"
  ```
- Send tmux key names:
  ```sh
  .agents/skills/roder-tmux/scripts/roder_tmux.sh keys -s roder Enter C-c
  ```
- Save pane output as both text and PNG:
  ```sh
  .agents/skills/roder-tmux/scripts/roder_tmux.sh capture -s roder -o .roder/tmux-captures
  ```
- Attach manually when visual inspection is useful:
  ```sh
  .agents/skills/roder-tmux/scripts/roder_tmux.sh attach -s roder
  ```
- Stop the session:
  ```sh
  .agents/skills/roder-tmux/scripts/roder_tmux.sh stop -s roder
  ```

## Workflow

1. Use a named tmux session per task, usually `roder` or `roder-<short-task>`.
2. Start from the repo root unless the user explicitly asks for another working directory.
3. Capture before sending disruptive keys like `C-c`, after every meaningful interaction, and before reporting results.
4. Report the saved `.txt` and `.png` paths in the final answer when captures are part of the requested evidence.

## Notes

- The helper targets pane `0.0` in the named session.
- PNG capture uses macOS `qlmanage` when available, or a Python Pillow renderer if Pillow is installed.
- If PNG rendering is unavailable, keep the `.txt` capture and state that PNG rendering could not run in the current environment.
