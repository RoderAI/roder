# Roder Agent-Swarm Mode

Agent-swarm mode lets a lead model (or you, via a slash command) launch many
similarly-shaped subagent tasks from one prompt template, watch bounded fanout,
resume unfinished children, and collect a single ordered, machine-readable
result. It is a composition layer over Roder's existing subagent dispatch — it
does not open a second agent runtime.

Swarm requires the subagent surface to be enabled (`[subagents].enabled = true`).
When it is, the `agent_swarm` tool is registered automatically.

## The `agent_swarm` tool

Input:

```json
{
  "description": "short description for the whole swarm",
  "subagent_type": "optional configured subagent type for every child",
  "prompt_template": "Read {{item}} and report one observation.",
  "items": ["src/a.rs", "src/b.rs"],
  "resume_agent_ids": { "existing-agent-id": "continue" }
}
```

Validation happens before any child is dispatched:

- `items` are trimmed and must contain at least two entries unless at least one
  `resume_agent_ids` entry is present.
- `prompt_template` is required whenever `items` are present and must contain the
  exact placeholder `{{item}}`.
- The rendered prompts must be distinct.
- Resumed children are dispatched before new item-based spawns.
- The total child count may not exceed `max_subagents` (default and hard cap
  `128`).
- `agent_swarm` should be the only tool call in a model response; run multiple
  swarms sequentially.

Result (durable transcript text plus structured `data.agent_swarm`):

```xml
<agent_swarm_result>
<summary>completed: 2, failed: 1, aborted: 0</summary>
<resume_hint>Call agent_swarm with resume_agent_ids using the agent_id values in this result to continue unfinished work.</resume_hint>
<subagent agent_id="..." item="src/a.rs" outcome="completed">...</subagent>
<subagent agent_id="..." item="src/b.rs" outcome="failed">...</subagent>
</agent_swarm_result>
```

Failed or aborted children that started running carry an `agent_id` so the lead
model can pass them back in `resume_agent_ids`.

> Note: the in-process dispatcher has no true "resume an existing agent id" path
> yet, so a resumed child is dispatched as a fresh run with the resume prompt and
> the new child thread id is surfaced as its `agent_id`.

## Scheduling

The scheduler (defaults shown) is bounded and deterministic:

- Start up to `initial_launch_limit = 5` children immediately, then launch one
  more every `launch_interval_ms = 700` ms while work remains.
- `max_concurrency` (unset by default) caps simultaneously-active children.
- `child_timeout_seconds` overrides the per-child dispatch timeout.
- Results are always returned in input order.
- On cancellation, completed children are preserved, started children are marked
  `aborted` with `state = started`, and never-started children are marked
  `aborted` with `state = not_started`.

### Provider rate limits

When the provider rate-limits a child, the swarm backs off on two levels:

- **Per child:** the child is retried with exponential backoff
  (`rate_limit_base_backoff_ms = 3000`, doubling: 3s, 6s, 12s, ...) up to
  `rate_limit_max_retries = 4` (hard-capped at 8) before it is reported
  `failed`.
- **Globally:** a shared rate-limit governor throttles the *whole* swarm so the
  children do not each hammer the provider in parallel. The first rate limit
  sizes a global capacity from the children that were active when it hit, then
  shrinks it by one. Sustained rate limits shrink capacity by one more (down to
  a floor of one) no more often than every
  `rate_limit_shrink_interval_ms = 2000`, and launches are paced apart while
  throttled. After a quiet window of `rate_limit_recovery_interval_ms = 180000`
  (three minutes) with no rate limit, the swarm recovers one unit of capacity so
  it speeds back up. The governor is inert until the first rate limit, so the
  normal ramp above is unaffected.

## Configuration

```toml
[agent_swarm]
max_subagents = 64        # clamped to 1..=128
initial_launch_limit = 5
launch_interval_ms = 700
max_concurrency = 4       # optional
child_timeout_seconds = 180  # optional
rate_limit_max_retries = 4            # per-child retries (hard cap 8)
rate_limit_base_backoff_ms = 3000     # per-child backoff base (doubles)
rate_limit_shrink_interval_ms = 2000  # min spacing between capacity shrinks
rate_limit_recovery_interval_ms = 180000  # quiet window before +1 capacity
```

Environment overrides (highest precedence; all parsed then clamped):

- `RODER_AGENT_SWARM_MAX_SUBAGENTS`
- `RODER_AGENT_SWARM_INITIAL_LAUNCH_LIMIT`
- `RODER_AGENT_SWARM_LAUNCH_INTERVAL_MS`
- `RODER_AGENT_SWARM_MAX_CONCURRENCY`
- `RODER_AGENT_SWARM_CHILD_TIMEOUT_SECONDS`
- `RODER_AGENT_SWARM_RATE_LIMIT_MAX_RETRIES`
- `RODER_AGENT_SWARM_RATE_LIMIT_BASE_BACKOFF_MS`
- `RODER_AGENT_SWARM_RATE_LIMIT_SHRINK_INTERVAL_MS`
- `RODER_AGENT_SWARM_RATE_LIMIT_RECOVERY_INTERVAL_MS`

## TUI commands

- `/agent-swarm` toggles persistent swarm mode (alias: `/swarm`).
- `/agent-swarm on` / `/agent-swarm off` force the state.
- `/agent-swarm status` reports the current state.
- `/agent-swarm <prompt>` runs one swarm task: it prepends a short swarm
  reminder to your prompt so the model reaches for `agent_swarm`, then submits.

While persistent swarm mode is on, the runtime injects the swarm reminder into
each turn's developer instructions server-side (your displayed transcript text
stays exactly as typed). Swarm mode never relaxes sandbox, capability, or
approval policy; children run through the already-authorized subagent dispatch
path. The model is held to the single-call rule by the runtime: a response that
mixes `agent_swarm` with other tools, or issues multiple swarms, is denied with
a retry message (see "Exclusivity" below).

## App-server / SDK

Swarm mode is runtime state, so any app-server or SDK client can drive it (not
just the TUI):

- `thread/set_agent_swarm_mode` — `{ "enabled": true, "trigger": "manual" }`
  returns `{ "enabled": true }`. `trigger` is `manual` (persistent toggle),
  `task` (one-shot), or `tool` (implicit `agent_swarm` entry).
- `settings/get` includes `"agentSwarmMode": <bool>`.
- An `AgentSwarmModeChanged` event is emitted when the mode toggles.

## Exclusivity

`agent_swarm` must be the only tool call in a model response. If a response
mixes it with other tools, or contains more than one `agent_swarm` call, the
runtime denies the whole batch and returns an error tool result (with actionable
retry text) for every call, so each `tool_call_id` is answered and the model
re-issues `agent_swarm` by itself.
