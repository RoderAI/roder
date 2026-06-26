## 0.1.2 (2026-06-26)

### Features

#### Agent-swarm mode

Add a Roder-native `agent_swarm` fanout tool and `/agent-swarm` (alias `/swarm`)
commands (roadmap phase 104). A lead model can launch many homogeneous
subagent tasks from one `prompt_template` (with the `{{item}}` placeholder) over
an `items` array, optionally resuming existing agents via `resume_agent_ids`,
and receives an ordered `<agent_swarm_result>` summary with completed/failed/
aborted counts and resumable agent ids. A bounded scheduler paces launches
(initial burst then one per interval), honors an optional concurrency cap,
preserves input order, and supports cooperative cancellation. Configure via
`[agent_swarm]` or `RODER_AGENT_SWARM_*` env. The `/agent-swarm on|off|status`
command toggles a persistent swarm reminder; `/agent-swarm <prompt>` runs one
swarm task.

#### Rate-limit-aware agent_swarm scheduling

Swarm children that fail with a provider rate limit are now requeued with
exponential backoff (default 3s, 6s, 12s, ... up to 4 retries) instead of
failing outright (roadmap 104, Task 3). The concurrency permit is held across
the backoff so a rate-limited swarm naturally throttles rather than hammering
the provider, and cancellation still wins promptly. Tunable via
`[agent_swarm].rate_limit_max_retries` / `rate_limit_base_backoff_ms` and the
`RODER_AGENT_SWARM_RATE_LIMIT_*` env vars (retries are clamped to a hard cap so
a swarm can never wait unboundedly).

#### Subagent and swarm children inherit the parent workspace

Subagent (`task`) and agent-swarm children built their tool-execution context
with no handles, so any child file/shell/search tool failed with "workspace
handle is not available" and the child could not do real work. Children now
inherit the lead turn's workspace, remote workspace, process runner, and
context-artifact handles via the new `SubagentDispatcher::dispatch_with_context`
(the parent goal controller and trace sink are intentionally not inherited).
Each child still runs on its own child thread/turn id, so it operates on the
same repository as an independent agent rather than being confused with the
main-line thread.

### Fixes

#### Enforce agent_swarm as the only tool call in a response

The core turn loop now denies any model response that mixes `agent_swarm` with
other tool calls, or issues multiple `agent_swarm` calls at once (roadmap 104,
Task 2). Each call in the offending batch gets an error tool result with
actionable retry text, so the model re-issues `agent_swarm` by itself and every
`tool_call_id` still receives a response (keeping chat-completions transcripts
valid). Adds `roder_api::subagents::agent_swarm_batch_violation` and the shared
`AGENT_SWARM_TOOL_NAME` constant.

#### Deterministic agent_swarm parallelism evidence

Add a barrier-based scheduler test proving independent swarm children run in
parallel during the normal launch ramp (roadmap 104, Task 6). Four children must
be simultaneously active to clear a 4-way barrier, giving flake-free offline
evidence that the swarm reduces wall-clock time versus serial execution.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
