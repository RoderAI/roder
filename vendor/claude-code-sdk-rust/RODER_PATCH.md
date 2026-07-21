# Roder patch provenance

This directory vendors `claude-code-sdk-rust` version 0.4.0 from
`https://github.com/PandelisZ/claude-agent-sdk-rust`, commit `e1cb00d`, under
its MIT license (`LICENSE`).

Roder-specific patch, initially 2026-07-15 and extended 2026-07-17:

- configure spawned Claude CLI children with `kill_on_drop(true)`;
- make `spawn_stream_message` stop when its receiver is dropped;
- always close/disconnect the owned client before the spawned stream task exits;
- expose `ClaudeAgentClient::run_client_stream` for hosts that need explicit
  ownership of the supervised stream lifecycle;
- add regressions proving receiver drop closes the transport and terminates then
  reaps an offline fake CLI child process.

The patch was published as `claude-code-sdk-rust` 0.4.1 on 2026-07-21. Roder
now consumes that registry release, so published downstream crates receive the
same supervised cleanup API without a workspace-local Cargo override. Keep this
copy as release provenance until the upstream repository history is reconciled.
