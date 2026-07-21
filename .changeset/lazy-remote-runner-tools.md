---
roder-core: minor
roder-tools: minor
---

# Lazily initialize per-thread remote runners and support one-shot exec

Runner-bound threads now create or resume their remote session only when an
approved native workspace tool first executes. Concurrent first tool calls
share one initialization, the live session is reused across later tools and
turns, and its state is persisted before the first tool runs so a new process
can rejoin it. Text-only and host-executed MCP turns do not wake a runner, and
failed initialization remains retryable without falling back to local tools.

`exec_command` now runs non-interactive one-shot commands through a remote
runner with remote working-directory scoping, shell/login handling, deadlines,
timeouts, output truncation, and the existing Codex-shaped result payload.
Remote TTY and stdin-continuation requests fail clearly instead of executing
on the host. Hosted runtimes can also disable local workspaces completely, so
a missing or malformed runner binding fails closed before any native workspace
tool can touch the host filesystem.
