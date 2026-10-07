# Roder Thread Goals

Roder goals are durable per-thread objectives shared by the runtime, app-server,
TUI, and model-facing goal tools. The behavior follows OpenAI Codex
`87be737b664` (`codex-rs/ext/goal`, state goal transitions, and TUI goal actions).

## Slash Command

- `/goal` shows the current objective, status, token usage, budget, and elapsed time.
- `/goal <objective>` starts a new active goal with fresh usage accounting. Replacing
  an unfinished goal asks for confirmation; cancelling keeps the existing goal.
- `/goal edit` pre-fills the composer with the current objective.
- `/goal edit <objective>` edits without resetting usage or the budget. Active,
  paused, blocked, and usage-limited goals keep their status. Editing a completed
  or budget-limited goal requests activation; an exhausted budget still prevents it.
- `/goal pause` pauses autonomous continuation.
- `/goal resume` resumes continuation, preserving usage. A goal already over its
  token budget stays budget-limited until the budget is increased or removed.
- `/goal clear` removes the goal.

The TUI footer shows the current goal status and a compact objective preview.

## Runtime Behavior

The runtime supplies the full objective, current budget, and Codex's continuation
instructions. Objectives are escaped as user-provided data. Continuation requires
concrete progress and an audit of every requirement against authoritative current
state before completion. A blocker must recur for at least three consecutive goal
turns before the model marks the goal blocked; resuming starts a fresh audit.

After a successful turn, an active goal starts another turn when its thread is
idle. Paused, blocked, budget-limited, usage-limited, completed, and cleared goals
stop automatic continuation. User interruption pauses the goal. A terminal
provider error blocks the goal, or marks it usage-limited for a typed usage/quota
failure. Three automatic turns without visible activity also block continuation.

Automatic continuation retains the admitted turn's instructions, developer
context, workspace, provider, model, reasoning, and service tier. It does not
replay user input or attachments, and a new explicit turn supplies its own
context. The runtime rechecks idle and active goal state under turn admission,
with goal mutations excluded until admission commits.

Conversation forks flush the source goal's in-flight progress and copy its
objective, status, budgets, usage, and timestamps into an independent child
snapshot. Clearing or updating the parent afterward does not alter the child.

Provider token usage is accounted as it arrives, including compaction and tokens
spent by descendant agents. Usage and elapsed time are flushed before status
changes and on turn termination. Work before creation or after completion/pause
is excluded, and replacing a goal cannot charge the new goal for old work.
Crossing a budget supplies wrap-up instructions to the in-flight turn and stops
continuation. Budget-limited takes precedence over pause/blocked, while a goal
that has actually been achieved can still be completed.

Goal mutations are serialized within a runtime and local goal files are replaced
atomically. Each update emits `thread/goal/updated`; clearing emits
`thread/goal/cleared`.

## Model Tools

- `get_goal` returns current status, usage, and remaining tokens.
- `create_goal` requires an explicit user or system/developer request. It succeeds
  only when no goal exists or the previous goal is complete. Paused, blocked,
  usage-limited, and budget-limited goals remain unfinished and cannot be replaced
  by the model. A token budget must be explicitly requested.
- `update_goal` accepts `complete`, `blocked`, or `paused`. Pause requires an
  explicit user request. Completion requires the full objective to be achieved;
  budget exhaustion or ending a turn does not establish completion. Resume,
  budgets, limit states, and clear remain user/system operations.

Completion returns current structured usage and, for budgeted or timed goals,
guidance to report it to the user. Both native `tools/call` data and the tool text
sent to inference providers include these fields.

## App-server and ACP

Native JSON-RPC clients use `thread/goal/get`, `thread/goal/set`, and
`thread/goal/clear`. For example, this pauses an existing goal:

```json
{"jsonrpc":"2.0","id":"pause","method":"thread/goal/set","params":{"threadId":"thread-id","status":"paused"}}
```

The goal tools use the same state:

```json
{"jsonrpc":"2.0","id":"pause-tool","method":"tools/call","params":{"threadId":"thread-id","toolName":"update_goal","arguments":{"status":"paused"}}}
```

ACP retains its existing advertised capabilities. Goal management uses Roder's
native API; it is not an advertised ACP slash-command capability. ACP
`session/cancel` still finishes `session/prompt` with `stopReason: "cancelled"`
and pauses the thread's active goal through the same runtime interruption path.

## Reference audit

The alignment corrects the following differences from the reference:

| Behavior | Previous Roder behavior | Aligned behavior |
| --- | --- | --- |
| Model creation | Replaced any inactive goal | Only absent or complete goals are replaceable |
| Model pause | Rejected | Allowed on explicit user request |
| Completion guidance | Short completion check | Full scope, evidence, fidelity, and blocker audits |
| Turn usage | Charged only after successful completion | Streamed usage survives interruption and errors |
| Completion report | Could precede current usage accounting | Flushes usage before returning completion |
| Delegated usage | Omitted from ancestor goals | Rolls up once through the hierarchy |
| Exhausted resume | Could reactivate over-budget goal | Remains budget-limited |
| Edit | Always resumed the goal | Preserves paused/blocked/usage-limited status |
| Replacement | Immediately cleared unfinished goal | Requires confirmation and validates before clearing |
| Empty responses | Could continue indefinitely | Blocks after three empty automatic turns |
| Continuation context | Reset turn configuration | Retains the admitted turn's configuration |
| Continuation admission | Used an earlier idle/status snapshot | Rechecks state atomically before launch |
| Conversation forks | Lost the source goal | Inherit an independent goal snapshot with current usage |

Roder retains its native goal storage and transport, rather than importing Codex's
SQLite extension host, telemetry, attachment UI, or Guardian infrastructure.
