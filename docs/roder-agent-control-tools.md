# Roder Agent Control Tools

Roder has two subagent surfaces:

- `task`: a one-shot child run that returns a bounded final result to the parent.
- Agent-control tools: independently running teammates backed by normal Roder
  threads and addressable for the rest of the parent session.

The agent-control tools are model-facing. App-server clients use the corresponding
`team/*` methods documented in `docs/app-server/api.md`.

| Tool | Purpose |
| --- | --- |
| `spawn_agent` | Create a teammate, start its first turn, and return its canonical task path. |
| `send_message` | Queue a message for an existing teammate without starting a new turn. |
| `followup_task` | Assign follow-up work and start or wake the teammate when it is idle. |
| `wait_agent` | Yield until a teammate finishes or the caller receives mailbox/steer activity. |
| `list_agents` | List `/root` and the accessible live task tree, optionally under a canonical path prefix. |
| `interrupt_agent` | Interrupt the teammate's active turn while keeping it available for later follow-up work. |

`close_agent` is not part of the model-facing contract. Interruption is deliberately
non-destructive: use `followup_task` after `interrupt_agent` to resume with a new
turn.

## Spawning and Context Inheritance

`spawn_agent` creates or reuses the caller's implicit team, creates a separate
Roder thread for the child, and starts its initial turn. The child has an
independent transcript and context window but works in the same workspace unless
the selected runner provides stronger isolation.

Example:

```json
{
  "task_name": "reviewer",
  "message": "Review the current patch and report blocking risks.",
  "agent_type": "release-audit",
  "fork_turns": "all"
}
```

Context and selection rules:

- `fork_turns: "all"` is the default and passes all surrounding parent turns
  available at spawn time. It inherits the parent live model, provider, and
  reasoning selection. `agent_type` is allowed as an advisory collaboration
  label (for example, `release-audit`); it does not select a model, provider,
  tool set, or configured `task` role. Full-history forks reject only `model`,
  `model_provider`, and `reasoning_effort` overrides.
- `fork_turns: "none"` starts with only the explicit child task and normal
  workspace/project instructions.
- A positive decimal string such as `"3"` passes only the most recent three
  parent turns.
- With `fork_turns: "none"` or a positive count, omit `model_provider`, `model`,
  and `reasoning_effort` to inherit the caller's live effective selection,
  including Ultra mode, or supply an explicit override supported by the chosen
  model.
- Forked turns are an initial snapshot. Later parent conversation is delivered
  only through `send_message` or `followup_task`.
- Forked conversation history is context, not the child's assignment. The
  newest `NEW_TASK` envelope carries the authoritative task for each child
  turn; parent orchestration visible in copied history must not be repeated
  unless that payload explicitly delegates it.

For example, a fresh-context specialist may override its model selection:

```json
{
  "task_name": "fast_search",
  "message": "Locate the relevant implementation and report file paths.",
  "fork_turns": "none",
  "agent_type": "explorer",
  "model_provider": "codex",
  "model": "gpt-5.6-sol",
  "reasoning_effort": "high"
}
```

Nested agents follow the same rules. A local task name is one canonical path
segment, so a lead at `/root` spawning `reviewer` creates `/root/reviewer`; if
that worker spawns `tests`, the child is `/root/reviewer/tests`. Roder supports
at most five nested agent levels below `/root`; a sixth-level spawn is rejected
before a team member or thread is created.

## Addressing and Messaging

Use the canonical task path returned by `spawn_agent` in its `task_name` result
field whenever possible. An
unambiguous relative task name is accepted within the caller's branch, but the
canonical `/root/...` path is required when the same local name appears in more
than one branch.

`send_message` and `followup_task` are intentionally different:

- `send_message` queues text for delivery at the running teammate's next message
  boundary. It does not start a turn for an idle or completed teammate.
- `followup_task` queues the instruction and starts a new turn when the teammate
  is idle or completed. If it is already running, the instruction is delivered
  at a safe message boundary.

Examples:

```json
{
  "target": "/root/reviewer",
  "message": "Also check the cancellation path."
}
```

```json
{
  "target": "/root/reviewer",
  "message": "Re-run the review after the latest patch."
}
```

Agent messages retain agent provenance and never count as user approval for a
permission prompt.

Model-visible inter-agent messages use typed envelopes with canonical paths:

```text
Message Type: NEW_TASK
Task name: /root/reviewer
Sender: /root
Payload:
Review the current patch and report blocking risks.
```

- `MESSAGE` is queued coordination from `send_message`.
- `NEW_TASK` is the authoritative assignment from `spawn_agent` or
  `followup_task`.
- `FINAL_ANSWER` is a terminal result delivered automatically to the worker's
  direct parent.

## Waiting and Completion Results

`wait_agent` yields for either a terminal result or new mailbox/user-steer
activity directed at the caller. Its `timeout_ms` accepts values from
10,000 milliseconds (10 seconds) through 3,600,000 milliseconds (one hour).
Omit a target to observe any accessible teammate; provide a canonical target to
watch one worker. A non-terminal wake returns `activity: "mailbox_or_steer"` and
the current agent statuses, allowing the next inference round to consume the
newly delivered message.

A terminal completion includes:

- `status`: completed, failed, interrupted, or closed.
- `final_message`: the worker's final assistant response when one was produced.
- `terminal_error`: the terminal failure text when the turn failed.
- stable team, member, thread, and canonical task identifiers. A live completion
  also includes its turn identifier; an already-stored terminal snapshot may
  omit the completed turn id.

`final_message` and `terminal_error` are independent optional fields. A provider
failure can include both partial final text and an error, while interruption may
include neither. App-server clients receive the same data on
`team/member/completed` as camel-case `finalMessage` and `error`.
The terminal result is also delivered automatically to the worker's direct
parent mailbox, so a waiting parent can continue from the next inference round
without polling for the final text.

## Interruption

`interrupt_agent` targets the active turn, not the teammate's durable identity.
The tool returns the previous status and leaves the canonical task path valid.
Use `followup_task` to assign new work afterward. This differs from deleting a
thread or cleaning up an app-server team.

When a provider returns multiple agent-control tool calls in one batch, Roder
executes the lifecycle-sensitive calls in model order so messaging, waiting, and
interruption remain deterministic.

The app-server team methods (`team/start`, `team/member/message`,
`team/member/interrupt`, and related methods) remain the external client control
plane over the same runtime.
