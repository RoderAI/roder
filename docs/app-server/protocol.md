# Roder App-Server Protocol

Roder's desktop-facing app-server contract uses desktop thread, turn, item,
model, filesystem, and command method names. This is the canonical app-server
surface for desktop, TUI, CLI, and sibling clients.

## Required Desktop Methods

| Method | Desktop caller | Roder backing behavior | Notes |
| --- | --- | --- | --- |
| `initialize` | startup handshake | Return app-server status, capabilities, cwd, active model, and provider metadata. | Startup entrypoint. |
| `thread/start` | new chat | Create a Roder thread/session with `model`, optional `modelProvider`, `cwd`, and `ephemeral`. | Returns a desktop `Thread`. |
| `thread/list` | sidebar bootstrap/refresh | List persisted Roder sessions as stable desktop `Thread` objects. | Pagination cursors are reserved. |
| `thread/read` | thread switch | Read a persisted thread by `threadId`; include turns/items when `includeTurns` is true. | Returns `thread: null` when not found. |
| `turn/start` | send prompt | Start a Roder turn on `threadId`, accepting desktop `input` text blocks and temporary `prompt` fallback. | Emits turn and item notifications. |
| `turn/steer` | steer active turn | Send additional user input to the active turn, enforcing `expectedTurnId` when provided. | Requires an active turn. |
| `turn/interrupt` | stop button | Interrupt the active turn for a thread; `turnId` is optional when there is a single active turn. | Uses the runtime interrupt path. |
| `model/list` | model picker | Return visible model descriptors with `id`, `name`, `modelProvider`, reasoning efforts, and default flags. | Desktop model-picker data. |
| `fs/readFile` | file preview | Read an absolute host path and return base64 bytes as `dataBase64`. | Read-only filesystem method. |
| `fs/readDirectory` | file browser | List direct children of an absolute host directory with `fileName`, `isDirectory`, and `isFile`. | Read-only filesystem method. |
| `command/exec` | one-off command runner | Run an argv vector with optional absolute `cwd`, env overrides, timeout, output cap, and optional `command/exec/outputDelta` streaming. | PTY, streaming stdin, resize, write, and terminate are deferred. |
| `team/start` | start an agent team | Create a lead thread plus long-lived teammate threads with `displayMode` `auto`, `in_process`, `tmux`, or `iterm2`. | Team control-plane methods use desktop singular method names. |
| `team/list` | team sidebar/bootstrap | List active or persisted teams as `TeamDescriptor` objects. | Supports optional `limit`; pagination cursors are reserved. |
| `team/read` | attach/split-pane bootstrap | Read a team plus persisted mailbox messages by `teamId`. | Used by `roder team attach --team ... --member ...`. |
| `team/member/start` | add teammate | Add a new long-lived teammate session to an existing team. | Returns the new `member` descriptor. |
| `team/member/message` | direct message | Start or steer the selected teammate's active turn and persist the mailbox message. | Does not inject hidden text into the lead transcript. |
| `team/member/interrupt` | stop focused teammate | Interrupt only the selected teammate's active turn. | `turnId` is accepted for client bookkeeping; the team member id is authoritative. |
| `team/member/focus` | headless focus acknowledgement | Validate that a member exists and echo the focused member id. | Split-pane focus is TUI-local; headless pane methods return a precise unsupported error. |
| `team/cleanup` | close team state | Remove persisted team state, refusing active members unless `force` is true. | Split-pane backends must close only panes they created. |
| `automations/status` | automation dashboard | Report scheduler enablement, store path, and due/running/leased counters. | Scheduling is app-server-owned and disabled by default. |
| `automations/list` | automation dashboard | List automation definitions for display and management. | Does not enable the scheduler. |
| `automations/runNow` | manual automation run | Queue one immediate automation occurrence through the normal run path. | Uses the same task/thread/turn audit path as scheduled runs. |
| `automations/runs` | automation history | Read run history, including failed and skipped missed runs. | Use `state` filtering for failed, running, or skipped views. |
| `automations/cancelRun` | stop automation run | Cancel a queued or running automation run. | Cancellation is run-id scoped. |

## Required Desktop Notifications

| Notification | Desktop reducer expectation | Roder source |
| --- | --- | --- |
| `thread/started` | `params.thread` is a desktop `Thread`; it becomes active and is inserted into the sidebar. | Session creation. |
| `turn/started` | `params.threadId` and `params.turn.id`; busy state becomes true. | Runtime turn start. |
| `item/started` | `params.item` with `type: "agentMessage"` or `tool.*`; creates in-progress visible rows. | Runtime assistant/tool start events. |
| `item/agentMessage/delta` | `threadId`, `turnId`, `itemId`, `delta`, optional `phase`; appends assistant text. | Inference text/commentary/reasoning deltas. |
| `item/completed` | `params.item` converts to one or more completed conversation messages. | Runtime assistant/tool completion. |
| `turn/completed` | `params.turn.id`; busy state clears when it matches the active turn. | Runtime turn completion. |
| `thread/status/changed` | `threadId`, `status`; sidebar status updates. `activeFlags` marks wait states such as `approvalRequired`, `userInputRequired`, and `planExitRequired`. | Runtime active/idle/wait state changes. |
| `session/approvalRequested` | `approvalId`, `toolId`, `toolName`, `threadId`, and `turnId`; clients should prompt and answer with `session/resolve_approval`. | Runtime tool policy approval request. |
| `session/approvalResolved` | `approvalId`, `approved`, `threadId`, and `turnId`; clients clear the approval prompt. | Runtime tool policy approval resolution. |
| `session/userInputRequested` | `requestId`, `questions`, `threadId`, and `turnId`; clients should prompt and answer with `session/resolve_user_input`. | Runtime `request_user_input` tool request. |
| `session/userInputResolved` | `requestId`, `answers`, `threadId`, and `turnId`; clients clear the input prompt. | Runtime user-input resolution. |
| `session/planExitRequested` | `requestId`, `targetMode`, optional `planSummary`, `threadId`, and `turnId`; clients should prompt and answer with `session/exit_plan`. | Runtime plan-mode exit request. |
| `session/planExitResolved` | `requestId`, `approved`, `targetMode`, `resolvedMode`, `threadId`, and `turnId`; clients clear the plan-exit prompt. | Runtime plan-mode exit resolution. |
| `command/exec/outputDelta` | `processId`, `stream`, `deltaBase64`, and `capReached`; appends streamed command output. | `command/exec` with `streamStdoutStderr: true`. |
| `team/started` | `params.team` is a `TeamDescriptor`; clients can render the roster. | `team/start`. |
| `team/member/started` | `teamId` plus a member descriptor; roster grows. | Runtime team member creation. |
| `team/member/statusChanged` | `teamId`, `memberId`, `status`; roster status updates. | Runtime member turn routing. |
| `team/member/messageDelta` | `teamId`, `memberId`, `turnId`, `delta`; append to that member's transcript. | Teammate inference deltas. |
| `team/member/completed` | `teamId`, `memberId`, optional `turnId`, `status`; selected teammate becomes idle/completed/interrupted. | Runtime turn completion/interruption. |
| `team/cleanupCompleted` | `teamId`, `forced`; remove local team state. | `team/cleanup`. |
| `automations/runStarted` | `run` with `state: "running"`; show active scheduled work. | Automation worker start. |
| `automations/runCompleted` | `run` with terminal success fields; mark the run complete. | Automation worker completion. |
| `automations/runFailed` | `run` plus `error`; show failed scheduled work. | Automation worker failure. |
| `automations/runSkipped` | `run` plus `reason`; show missed or suppressed work. | Scheduler missed-run/catch-up handling. |
| `automations/needsInput` | `run` plus `error`; prompt that automation could not proceed unattended. | Automation worker approval or user-input wait. |

Automation clients should treat scheduler state as process-local. Desktop may
launch an app-server with scheduler enablement; a TUI-local app-server should
only read and manage automations unless explicitly launched with scheduler
flags. See `docs/app-server/automations.md` for method payloads, missed-run
behavior, and lease recovery.

## Team Display Modes

`displayMode: "in_process"` keeps all members inside one TUI. Shift+Down cycles
lead and teammates, Shift+Up cycles backward where the terminal reports modified
arrow keys, and the composer sends to the focused member's thread.

`displayMode: "tmux"` creates one pane per teammate when `$TMUX` is present and
the configured `tmux` command is available. Each pane runs:

```sh
roder team attach --team <team-id> --member <member-id>
```

`displayMode: "iterm2"` uses `it2` only when `TERM_PROGRAM=iTerm.app` and the
configured command is available. `displayMode: "auto"` prefers tmux, then iTerm2,
then falls back to in-process with a concrete reason. Headless app-server clients
are never forced to use tmux or iTerm2.

## Deferred Follow-On Surface

`command/exec/write`, `command/exec/terminate`, `command/exec/resize`, and PTY
mode are still deferred. Roder returns a precise unsupported or
method-not-found JSON-RPC error for those surfaces rather than silently
accepting a no-op.

## Input Notes

`turn/start` and `turn/steer` should prefer:

```json
{
  "threadId": "thread-id",
  "input": [{ "type": "text", "text": "inspect this repo" }]
}
```

Clients should send `input` blocks for new code. `prompt` remains an accepted
field on the canonical method while richer input blocks are rolled through the
clients.
