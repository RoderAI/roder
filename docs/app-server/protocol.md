# Roder App-Server Protocol

Roder's app-server contract uses protocol thread, turn, item, model,
filesystem, and command method names. This is the canonical app-server surface
for app, TUI, CLI, SDK, and sibling clients.

## Required Client Methods

| Method | Client caller | Roder backing behavior | Notes |
| --- | --- | --- | --- |
| `initialize` | startup handshake | Return app-server status, capabilities, cwd, active model, and provider metadata. | Startup entrypoint. |
| `thread/start` | new chat | Create a Roder thread with typed `selection` or legacy `model`, optional `modelProvider`, optional `reasoning`, required absolute `cwd`, and `ephemeral`. | Returns a protocol `Thread` with `selectionMode`; missing, empty, or relative cwd is rejected. |
| `thread/list` | sidebar bootstrap/refresh | List persisted Roder threads as stable protocol `Thread` objects. | Pagination cursors are reserved. |
| `thread/read` | thread switch | Read a persisted thread by `threadId`; include turns/items when `includeTurns` is true. | Returns `thread: null` when not found. |
| `thread/archive` | archive/delete thread action | Archive a persisted thread and remove in-memory protocol state for that thread. | `thread/list` no longer returns archived threads. |
| `turn/start` | send prompt | Start a Roder turn on `threadId`, or queue same-turn steering when a turn is already active. Accepts protocol `input` text blocks and temporary `prompt` fallback. | Emits turn and item notifications for new turns; active-turn steering continues the existing turn. |
| `turn/steer` | steer active turn | Send additional user input to the active turn, enforcing `expectedTurnId` when provided. | Requires an active turn. |
| `turn/interrupt` | stop button | Interrupt the active turn for a thread; `turnId` is optional when there is a single active turn. | Uses the runtime interrupt path. |
| `model/list` | model picker | Return visible model descriptors with `id`, `name`, `modelProvider`, reasoning efforts, and default flags. | Protocol model-picker data. |
| `model/select` | model picker | Select Manual provider/model/reasoning or a configured Auto routing option. | Manual bypasses routing; Auto routes normal turns through the selected router option. |
| `workspace/files/status` | file tree/search bootstrap | Read app-server-owned file-index state for a workspace and optional root. | Returns `missing`, `building`, `ready`, `stale`, or `failed`. |
| `workspace/files/rebuild` | file tree/search refresh | Build or refresh the cached workspace file index. | Emits `workspace/files/statusChanged`; cache is keyed by canonical root, not selected root order. |
| `workspace/files/children` | file tree expansion | List registered workspace roots or direct children under a root-relative directory. | Canonical file-tree method; paths are relative and scoped to registered roots. |
| `workspace/files/query` | quick-open and mentions | Ranked fuzzy match over indexed workspace files and directories. | Result limits do not cap the underlying index. |
| `workspace/files/read` | file preview | Read bounded UTF-8 text, binary metadata, or unsupported-encoding metadata for an indexed workspace file. | Use with `rootId` and root-relative `path`. |
| `fs/readFile` | low-level host file read | Read an absolute host path and return base64 bytes as `dataBase64`. | Not the workspace file-preview API. |
| `fs/readDirectory` | low-level host directory read | List direct children of an absolute host directory with `fileName`, `isDirectory`, and `isFile`. | File browsers should not recursively call this; use `workspace/files/children`. |
| `command/exec` | one-off command runner | Run an argv vector with optional absolute `cwd`, env overrides, timeout, output cap, and optional `command/exec/outputDelta` streaming. | PTY, streaming stdin, resize, write, and terminate are deferred. |
| `processes/list`, `processes/get`, `processes/stop`, `processes/stopAll`, `processes/subscribe` | process monitor | Inspect and stop Roder-owned command, task, and remote-runner processes. | Does not enumerate arbitrary host OS processes. |
| `skills/list`, `skills/read`, `skills/setEnabled`, `skills/setExposure` | skills manager | List/read skill descriptors and persist canonical enablement/exposure rules. | Mutating by ambiguous skill name returns an invalid-params error; select by canonical path. |
| `search_index/status`, `search_index/warmup`, `search_index/rebuild`, `search_index/clear` | search-index dashboard | Manage the persistent regex index for a workspace. | Controlled by `settings/set_search_index`; emits `search_index/statusChanged`. |
| `index/status`, `index/rebuild`, `index/search`, `index/readChunk`, `index/proofs/list` | semantic code-index inspector | Build/query proof-verified code chunks and read chunk source only with `includeSource: true`. | Emits `index/statusChanged` after rebuild. |
| `inference/routing/status`, `inference/routing/metrics` | inference routing diagnostics | Inspect latest adaptive routing status, selected-versus-baseline estimates, and regret counters. | Read-only event-derived diagnostics; estimates are not exact provider billing. |
| `retrieval/recommendations`, `retrieval/metrics`, `retrieval/promoted` | retrieval diagnostics | Inspect route recommendations, outcomes, and promoted capability state for a turn. | Diagnostic surface derived from runtime retrieval events. |
| `eval/reports/list`, `eval/report/read` | eval report viewer | List and read bounded markdown reports from `<workspace>/evals/reports`. | Report ids must come from the list response. |
| `team/start` | start an agent team | Create a lead thread plus long-lived teammate threads with `displayMode` `auto`, `in_process`, `tmux`, or `iterm2`. | Team control-plane methods use singular protocol method names. |
| `team/list` | team sidebar/bootstrap | List active or persisted teams as `TeamDescriptor` objects. | Supports optional `limit`; pagination cursors are reserved. |
| `team/read` | attach/split-pane bootstrap | Read a team plus persisted mailbox messages by `teamId`. | Each message includes `kind`: `MESSAGE`, `NEW_TASK`, or `FINAL_ANSWER`. |
| `team/member/start` | add teammate | Add a new long-lived teammate thread to an existing team. | Returns the new `member` descriptor. |
| `team/member/message` | direct message | Start or steer the selected teammate's active turn and persist the mailbox message. | Does not inject hidden text into the lead transcript. |
| `team/member/interrupt` | stop focused teammate | Interrupt only the selected teammate's active turn. | `turnId` is accepted for client bookkeeping; the team member id is authoritative. |
| `team/member/focus` | headless focus acknowledgement | Validate that a member exists and echo the focused member id. | Split-pane focus is TUI-local; headless pane methods return a precise unsupported error. |
| `team/cleanup` | close team state | Remove persisted team state, refusing active members unless `force` is true. | Split-pane backends must close only panes they created. |
| `automations/status` | automation dashboard | Report scheduler enablement, store path, and due/running/leased counters. | Scheduling is app-server-owned and disabled by default. |
| `automations/list` | automation dashboard | List automation definitions for display and management. | Does not enable the scheduler. |
| `automations/runNow` | manual automation run | Queue one immediate automation occurrence through the normal run path. | Uses the same task/thread/turn audit path as scheduled runs. |
| `automations/runs` | automation history | Read run history, including failed and skipped missed runs. | Use `state` filtering for failed, running, or skipped views. |
| `automations/cancelRun` | stop automation run | Cancel a queued or running automation run. | Cancellation is run-id scoped. |

Thread metadata is required to carry an absolute workspace. The app-server
projects that workspace into `Thread.cwd`; it does not synthesize a fallback cwd
from the app-server process when persisted metadata is missing or invalid.

## Required Client Notifications

| Notification | Client reducer expectation | Roder source |
| --- | --- | --- |
| `thread/started` | `params.thread` is a protocol `Thread`; it becomes active and is inserted into the thread list. | Thread creation. |
| `turn/started` | `params.threadId` and `params.turn.id`; busy state becomes true. | Runtime turn start. |
| `item/started` | Full `ThreadItemEvent` envelope with `event.type: "itemStarted"` and a typed `event.item`; creates an in-progress canonical item. | Recorded public item event. |
| `item/agentMessage/delta` | Full `ThreadItemEvent` envelope with `event.type: "itemDelta"` and `delta.type: "agentMessageText"`; appends assistant text to `event.itemId`. | Recorded public item event. |
| `item/reasoning/textDelta` | Full `ThreadItemEvent` envelope with `delta.type: "reasoningText"`; appends reasoning content to `event.itemId`. | Recorded public item event. |
| `item/completed` | Full `ThreadItemEvent` envelope with `event.type: "itemCompleted"` and a typed `event.item`; completes the existing canonical item. Inference routing decisions complete a persisted `routingDecision` item. | Recorded public item event. |
| `turn/completed` | `params.turn.id`; busy state clears when it matches the active turn. | Runtime turn completion. |
| `thread/status/changed` | `threadId`, `status`; sidebar status updates. `activeFlags` marks wait states such as `approvalRequired`, `userInputRequired`, and `planExitRequired`. | Runtime active/idle/wait state changes. |
| `thread/approvalRequested` | `approvalId`, `toolId`, `toolName`, `threadId`, and `turnId`; clients should prompt and answer with `thread/resolve_approval`. | Runtime tool policy approval request. |
| `thread/approvalResolved` | `approvalId`, `approved`, `threadId`, and `turnId`; clients clear the approval prompt. | Runtime tool policy approval resolution. |
| `thread/userInputRequested` | `requestId`, `questions`, `threadId`, and `turnId`; clients should prompt and answer with `thread/resolve_user_input`. | Runtime `request_user_input` tool request. |
| `thread/userInputResolved` | `requestId`, `answers`, `threadId`, and `turnId`; clients clear the input prompt. | Runtime user-input resolution. |
| `thread/planExitRequested` | `requestId`, `targetMode`, optional `planSummary`, `threadId`, and `turnId`; clients should prompt and answer with `thread/exit_plan`. | Runtime plan-mode exit request. |
| `thread/planExitResolved` | `requestId`, `approved`, `targetMode`, `resolvedMode`, `threadId`, and `turnId`; clients clear the plan-exit prompt. | Runtime plan-mode exit resolution. |
| `command/exec/outputDelta` | `processId`, `stream`, `deltaBase64`, and `capReached`; appends streamed command output. | `command/exec` with `streamStdoutStderr: true`. |
| `process.started`, `process.output`, `process.exited`, `process.stopping`, `process.stopped`, `process.failed` | Process descriptor/output payloads; refresh process monitor rows and output tails. | Roder process registry. |
| `skills/catalogLoaded`, `skills/configApplied`, `skills/activationResolved`, `skills/indexRendered`, `skills/invoked`, `skills/autoActivated`, `skills/skipped` | Update skills manager or diagnostics panels. | Runtime skills registry and injection paths. |
| `search_index/statusChanged` | `{ status }`; refresh regex search-index state. | Search-index status, warmup, rebuild, clear, and setting changes. |
| `index/statusChanged` | `{ status }`; refresh semantic code-index state. | Code-index rebuild. |
| `retrieval/routePlanned`, `retrieval/routeAccepted`, `retrieval/routeIgnored`, `retrieval/routeFailed`, `retrieval/resultUsed`, `retrieval/discoveryItemPromoted`, `retrieval/promotionSkipped` | Update retrieval diagnostics for a turn. | Runtime retrieval router. |
| `team/started` | `params.team` is a `TeamDescriptor`; clients can render the roster. | `team/start`. |
| `team/member/started` | `teamId` plus a member descriptor; roster grows. | Runtime team member creation. |
| `team/member/statusChanged` | `teamId`, `memberId`, `status`; roster status updates. | Runtime member turn routing. |
| `team/member/messageDelta` | `teamId`, `memberId`, `turnId`, `delta`; append to that member's transcript. | Teammate inference deltas. |
| `team/member/completed` | `teamId`, `memberId`, optional `turnId`, `status`, optional `finalMessage`, and optional `error`; selected teammate becomes idle/completed/interrupted and its terminal result becomes available for synthesis. | Runtime turn completion/interruption. |
| `team/cleanupCompleted` | `teamId`, `forced`; remove local team state. | `team/cleanup`. |
| `automations/runStarted` | `run` with `state: "running"`; show active scheduled work. | Automation worker start. |
| `automations/runCompleted` | `run` with terminal success fields; mark the run complete. | Automation worker completion. |
| `automations/runFailed` | `run` plus `error`; show failed scheduled work. | Automation worker failure. |
| `automations/runSkipped` | `run` plus `reason`; show missed or suppressed work. | Scheduler missed-run/catch-up handling. |
| `automations/needsInput` | `run` plus `error`; prompt that automation could not proceed unattended. | Automation worker approval or user-input wait. |

Automation clients should treat scheduler state as process-local. App clients may
launch an app-server with scheduler enablement; a TUI-local app-server should
only read and manage automations unless explicitly launched with scheduler
flags. See `docs/app-server/automations.md` for method payloads, missed-run
behavior, and lease recovery.

## Schema Manifest

The current method manifest is checked in at
`schemas/app-server/roder-app-server.v1.json`, and
`schemas/app-server/methods.schema.json` describes the manifest shape. App
and sibling clients that generate low-level method helpers should use those
files rather than scraping `docs/app-server/api.md`.

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

Model picker clients should call `providers/list` for real providers plus the
`routingOptions` sibling list. Auto options are selected with `model/select`
using `{ "type": "auto", "optionId": "..." }`; they are not fake provider or
model ids.
