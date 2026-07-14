---
roder-tools: minor
---

# `unified_exec` tool for Codex tool-shape parity

Adds `unified_exec`, a single-tool wrapper over the same `ExecSessionManager`
that already backs `exec_command`/`write_stdin`, matching Codex's persistent
PTY tool shape that gpt-5.5 was RL-trained on: `{ input, session_id?,
timeout_ms? }`. Omitting `session_id` starts a new session running `input` as
a shell command; passing one writes `input` to that session's stdin. Both
cases return output collected up to `timeout_ms` (default 1000ms) plus the
session ID if the command is still running — `timeout_ms` bounds the wait for
output, it does not kill the process. `session_id` is accepted and returned as
a string, matching Codex's wire shape, and a plain integer is still accepted
on input. `exec_command` and `write_stdin` stay registered and unchanged; all
three tools share the same session pool, so a session started by one can be
driven by another.
