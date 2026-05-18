# Roder App-Server Protocol

Roder's desktop-facing app-server contract uses Codex-style thread, turn, item,
model, filesystem, and command method names. Older Roder-only method names may
remain as internal compatibility aliases for the TUI while desktop clients move
to the canonical surface below.

## Required Desktop Methods

| Method | Desktop caller | Roder backing behavior | Compatibility note |
| --- | --- | --- | --- |
| `initialize` | startup handshake | Return app-server status, capabilities, cwd, active model, and provider metadata. | Replaces `system/initialize` as the desktop startup entrypoint. |
| `thread/start` | new chat | Create a Roder thread/session with `model`, optional `modelProvider`, `cwd`, and `ephemeral`. | Replaces `sessions/create` for desktop. |
| `thread/list` | sidebar bootstrap/refresh | List persisted Roder sessions as stable desktop `Thread` objects. | Replaces `sessions/list` for desktop. |
| `thread/read` | thread switch | Read a persisted thread by `threadId`; include turns/items when `includeTurns` is true. | Replaces `sessions/load` for desktop. |
| `turn/start` | send prompt | Start a Roder turn on `threadId`, accepting Codex-style `input` text blocks and temporary `prompt` fallback. | Replaces `turns/start` for desktop. |
| `turn/steer` | steer active turn | Send additional user input to the active turn, enforcing `expectedTurnId` when provided. | Replaces `turns/steer` for desktop. |
| `turn/interrupt` | stop button | Interrupt the active turn for a thread; `turnId` is optional when there is a single active turn. | Replaces `turns/interrupt` for desktop. |
| `model/list` | model picker | Return visible model descriptors with `id`, `name`, `modelProvider`, reasoning efforts, and default flags. | Replaces `providers/list` for desktop model data. |
| `fs/readFile` | file preview | Read an absolute host path and return base64 bytes as `dataBase64`. | Implemented as a read-only filesystem compatibility method. |
| `fs/readDirectory` | file browser | List direct children of an absolute host directory with `fileName`, `isDirectory`, and `isFile`. | Implemented as a read-only filesystem compatibility method. |
| `command/exec` | one-off command runner | Run an argv vector with optional absolute `cwd`, env overrides, timeout, output cap, and optional `command/exec/outputDelta` streaming. | PTY, streaming stdin, resize, write, and terminate are deferred. |
| `team/start` | start an agent team | Create a lead thread plus long-lived teammate threads with `displayMode` `auto`, `in_process`, `tmux`, or `iterm2`. | Team control-plane methods use Codex-style singular names. |
| `team/list` | team sidebar/bootstrap | List active or persisted teams as `TeamDescriptor` objects. | Supports optional `limit`; pagination cursors are reserved. |
| `team/read` | attach/split-pane bootstrap | Read a team plus persisted mailbox messages by `teamId`. | Used by `roder team attach --team ... --member ...`. |
| `team/member/start` | add teammate | Add a new long-lived teammate session to an existing team. | Returns the new `member` descriptor. |
| `team/member/message` | direct message | Start or steer the selected teammate's active turn and persist the mailbox message. | Does not inject hidden text into the lead transcript. |
| `team/member/interrupt` | stop focused teammate | Interrupt only the selected teammate's active turn. | `turnId` is accepted for client bookkeeping; the team member id is authoritative. |
| `team/member/focus` | headless focus acknowledgement | Validate that a member exists and echo the focused member id. | Split-pane focus is TUI-local; headless pane methods return a precise unsupported error. |
| `team/cleanup` | close team state | Remove persisted team state, refusing active members unless `force` is true. | Split-pane backends must close only panes they created. |

## Required Desktop Notifications

| Notification | Desktop reducer expectation | Roder source |
| --- | --- | --- |
| `thread/started` | `params.thread` is a desktop `Thread`; it becomes active and is inserted into the sidebar. | Session creation. |
| `turn/started` | `params.threadId` and `params.turn.id`; busy state becomes true. | Runtime turn start. |
| `item/started` | `params.item` with `type: "agentMessage"` or `tool.*`; creates in-progress visible rows. | Runtime assistant/tool start events. |
| `item/agentMessage/delta` | `threadId`, `turnId`, `itemId`, `delta`, optional `phase`; appends assistant text. | Inference text/commentary/reasoning deltas. |
| `item/completed` | `params.item` converts to one or more completed conversation messages. | Runtime assistant/tool completion. |
| `turn/completed` | `params.turn.id`; busy state clears when it matches the active turn. | Runtime turn completion. |
| `thread/status/changed` | `threadId`, `status`; sidebar status updates. | Runtime active/idle state changes. |
| `command/exec/outputDelta` | `processId`, `stream`, `deltaBase64`, and `capReached`; appends streamed command output. | `command/exec` with `streamStdoutStderr: true`. |
| `team/started` | `params.team` is a `TeamDescriptor`; clients can render the roster. | `team/start`. |
| `team/member/started` | `teamId` plus a member descriptor; roster grows. | Runtime team member creation. |
| `team/member/statusChanged` | `teamId`, `memberId`, `status`; roster status updates. | Runtime member turn routing. |
| `team/member/messageDelta` | `teamId`, `memberId`, `turnId`, `delta`; append to that member's transcript. | Teammate inference deltas. |
| `team/member/completed` | `teamId`, `memberId`, optional `turnId`, `status`; selected teammate becomes idle/completed/interrupted. | Runtime turn completion/interruption. |
| `team/cleanupCompleted` | `teamId`, `forced`; remove local team state. | `team/cleanup`. |

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

The temporary desktop shim may still send `{ "prompt": "..." }` when no rich
input blocks are present. Roder accepts that only as a transition fallback.
