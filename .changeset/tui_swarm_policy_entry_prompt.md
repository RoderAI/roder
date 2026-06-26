---
roder-tui: minor
---

# Prompt to switch policy mode when entering agent-swarm

When agent-swarm mode is enabled from an approval-gating policy mode
(`Default` or `Plan`), the TUI now offers a one-key confirm dialog to switch to
`Accept All` so swarm children don't each stall on a separate tool approval
(roadmap 104, Task 4). Confirming maps to the existing `thread/set_mode`
(`Accept All`); declining keeps the current mode and explains that children will
wait for per-tool approval. Non-gating modes (`Accept All`, `Bypass`) and any
already-open modal suppress the prompt.
