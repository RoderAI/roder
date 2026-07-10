---
roder-api: minor
roder-core: minor
roder-protocol: minor
roder-app-server: minor
roder-tui: patch
roder: patch
---

# Match Codex V2 Ultra agent lifecycle semantics

### Added

- Added Codex V2-style canonical agent trees, full/empty/last-N context forks,
  nested agents, reusable follow-up turns, mailbox-aware waiting, and
  non-destructive interruption.
- Added exact parent model, provider, Ultra reasoning, workspace, policy, tool,
  runner, and live developer-context inheritance for spawned agents.
- Added full `team/started`, `team/member/started`, and terminal result details
  to the app-server protocol.

### Changed

- `send_message` now queues coordination without starting an idle agent, while
  `followup_task` starts or steers the existing canonical agent thread.
- Child final results and terminal errors are delivered automatically to their
  direct parent, and completed identities remain available for later work.
- Inter-agent delivery now uses typed `MESSAGE`, `NEW_TASK`, and `FINAL_ANSWER`
  envelopes with canonical sender and recipient paths.

### Fixed

- Fixed spawn-capacity, completion/follow-up, interruption, mailbox batching,
  acknowledgement, restart, and wait races found by comparison with Codex V2
  and Claude Code agent workflows.
- Prevented full-history children from replaying parent orchestration by making
  the newest `NEW_TASK` payload the authoritative child assignment.
- Preserved spawn-time live instructions, developer context, and model
  selection across reusable follow-up turns.
- Prevented interrupted-turn mailbox reservations from stranding queued
  messages or accepting stale delivery acknowledgements.
- Bounded recursive agent paths to five levels below `/root`, rejecting deeper
  spawns before creating team or thread state.
