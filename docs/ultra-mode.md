# Roder Ultra Mode

Ultra mode turns on **proactive multi-agent delegation** for the current thread
(or the whole runtime). It is a first-class Roder mode, available for **any**
model — not only Codex Sol/Terra Ultra reasoning effort.

When ultra mode is on, the runtime injects the Codex-style multi-agent policy
into each turn's developer instructions: the model should spawn sub-agents when
parallel work would materially improve speed or quality, and should follow the
agent-control workflow (`spawn_agent`, `send_message`, `followup_task`,
`wait_agent`, `list_agents`, `interrupt_agent`). Your displayed transcript text
stays exactly as typed; the policy is applied server-side so every
app-server/SDK client benefits.

Ultra mode does **not** relax sandbox, capability, or approval policy. Children
run through the existing agent-control and subagent dispatch paths.

## Relationship to Sol/Terra Ultra effort

On Codex GPT-5.6 Sol and Terra, the catalog still exposes an **Ultra** reasoning
effort. Selecting Ultra effort:

1. Keeps Ultra visible as the selected effort in Roder
2. Maps to the provider's `max` wire effort
3. Enables proactive multi-agent for that model (same policy as ultra mode)

Ultra mode is independent:

| State | Effect |
| --- | --- |
| `/ultra on` (any model) | Proactive multi-agent policy |
| Sol/Terra + Ultra effort | Proactive multi-agent + max wire effort |
| Sol/Terra + lower effort, ultra mode off | Explicit-request-only multi-agent policy |
| Other models, ultra mode off | No multi-agent policy injection |

You can run Ultra effort on Sol/Terra without using `/ultra`, or use `/ultra`
with Grok, Claude, Luna, or any other model without changing reasoning effort.

## TUI commands

- `/ultra` toggles persistent ultra mode.
- `/ultra on` / `/ultra off` force the state.
- `/ultra status` reports the current state.
- `/ultra <prompt>` runs one ultra task: it prepends a short ultra reminder to
  your prompt so the model reaches for multi-agent tools, then submits (without
  flipping persistent mode).

While persistent ultra mode is on, the composer title shows **Ultra** next to
the policy mode (for example `Bypass - Ultra`, or `Bypass - Ultra · Agent Swarm`
when agent-swarm mode is also on).

## App-server / SDK

Ultra mode is runtime state, so any app-server or SDK client can drive it:

- `thread/set_ultra_mode` — `{ "enabled": true, "trigger": "manual" }` returns
  `{ "enabled": true }`. Optional `threadId` scopes the toggle to one thread.
  `trigger` is `manual` (persistent toggle) or `task` (one-shot).
- `settings/get` includes `"ultraMode": <bool>` (the runtime-global default).
- An `UltraModeChanged` event is emitted when the mode toggles; app-server
  clients also receive an `ultra/modeChanged` notification.

## Agent-swarm mode

Ultra mode and agent-swarm mode compose:

- **Ultra** encourages proactive multi-agent use of `spawn_agent` / agent-control
  tools for general parallel work.
- **Agent-swarm** (`/agent-swarm`) nudges the model toward the homogeneous
  `agent_swarm` fanout tool for many similarly-shaped items.

Enable both when a large task needs both open-ended teammates and template-based
fanout. See [`docs/agent-swarm-mode.md`](./agent-swarm-mode.md).

## Notes

- Ultra mode can increase provider/subscription usage because the model is
  allowed to spawn more concurrent agents.
- Sol/Terra Ultra effort still maps to `max` on the wire; ultra mode alone does
  not change reasoning effort for models that do not advertise Ultra.
