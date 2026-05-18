# Roder App-Server API

This document is the canonical integrator-facing reference for the Roder
app-server API. It describes the JSON-RPC methods implemented by
`crates/roder-app-server`, the shared wire DTOs in `crates/roder-protocol`, and
the notification stream emitted to desktop or sibling clients.

> Maintenance note: update this document with the `roder-app-server-docs` skill
> whenever app-server methods, request/response types, events, auth/config
> behavior, provider/model behavior, or session/thread semantics change.

## Overview

The app-server is a JSON-RPC 2.0 control plane for Roder runtime state. Clients
use it to:

- initialize against the current runtime, provider, model, workspace, and
  settings.
- create or resume sessions and desktop-shaped threads.
- start, steer, interrupt, and observe turns.
- list/select providers, models, runners, tools, agents, commands, memories,
  media artifacts, workflow imports, plan reviews, hunks, and background tasks.
- receive notifications for turn lifecycle, streamed assistant output, tool
  lifecycle, teams, workflow imports, media, memory, plan review, and hunk
  events.

The source of truth for method registration is
`AppServer::handle_request` in `crates/roder-app-server/src/server.rs`.

## Transport

All methods use JSON-RPC 2.0 request/response envelopes:

```json
{
  "jsonrpc": "2.0",
  "id": "request-id",
  "method": "thread/start",
  "params": {}
}
```

Responses echo the request `id` and contain either `result` or `error`:

```json
{
  "jsonrpc": "2.0",
  "id": "request-id",
  "result": {}
}
```

Local clients can call the in-process `LocalAppClient`. Remote mode exposes the
same JSON-RPC method surface over an authenticated WebSocket; see
`docs/app-server/remote.md` for startup flags, bearer-token headers,
subprotocol auth, pairing URLs, and remote security assumptions.

Notifications are JSON-RPC notification envelopes with no `id`:

```json
{
  "jsonrpc": "2.0",
  "method": "turn/started",
  "params": {}
}
```

## Authentication and Credentials

Most app-server methods run with the local Roder process authority. There is no
per-request app-server auth layer for in-process clients.

Remote WebSocket mode requires a bearer token during the WebSocket handshake.
Clients send either:

```text
Authorization: Bearer <token>
```

or, for browser-constrained clients:

```text
Sec-WebSocket-Protocol: gode.remote.v1, bearer.<token>
```

Provider auth is provider-specific:

- `auth/codex/login`, `auth/codex/status`, and `auth/codex/logout` manage the
  Codex OAuth token store through `roder-codex-auth`.
- `providers/list` reports each provider's `authType`, `authLabel`,
  `authenticated`, and optional `authDetail`.
- API-key providers rely on environment/config outside this app-server method
  surface.

Config persistence is opt-in on the `AppServer` instance. When enabled,
`providers/select`, `settings/set_web_search`, and `settings/set_default_mode`
write the selected defaults to `~/.roder/config.toml`.

## Core Concepts

`session` is the persisted runtime conversation unit used by the Roder runtime.
App-server clients interact with sessions through the desktop-facing `thread/*`
methods.

`thread` is the desktop-facing view of a Roder session. It is shaped as:

```json
{
  "id": "thread-123",
  "sessionId": "thread-123",
  "preview": "Untitled thread",
  "modelProvider": "openai",
  "createdAt": 1770000000,
  "updatedAt": 1770000100,
  "status": { "type": "idle", "activeFlags": [] },
  "cwd": "/Users/pz/w/gode",
  "name": "optional title",
  "turns": []
}
```

`turn` is one model interaction within a thread:

```json
{
  "id": "turn-123",
  "items": [],
  "itemsView": "default",
  "status": "inProgress",
  "startedAt": 1770000000
}
```

`item` is a visible row or event within a turn. Known item `type` values include
`userMessage`, `agentMessage`, `reasoning`, `toolCall`, `toolMessage`,
`tool.<name>`, and `raw`.

`provider` is an inference backend. Provider/model notation is exposed as a
provider id plus model id, for example `openai` and `gpt-5.5`, or provider
catalog entries that intentionally use Codex provider IDs.

`mode` is Roder's policy mode. App-server clients see it in `session/get` and
can change it with `session/set_mode` or `settings/set_default_mode`.

## Method Index

Core:

| Method | Purpose |
| --- | --- |
| `initialize` | Desktop startup handshake with active provider, model, and cwd. |
| `extensions/list` | List extension manifests and capability status. |
| `providers/list` | List providers, auth status, capabilities, and models. |
| `providers/configure` | Persist an API key for an API-key provider. |
| `providers/select` | Select active default provider/model/reasoning. |
| `model/list` | List desktop model descriptors. |
| `settings/get` | Read hosted web search mode and default policy mode. |
| `settings/set_web_search` | Set hosted web search mode. |
| `settings/set_default_mode` | Set default policy mode. |
| `auth/codex/login` | Start Codex OAuth login. |
| `auth/codex/status` | Read Codex OAuth status. |
| `auth/codex/logout` | Clear Codex OAuth credentials. |
| `auth/supergrok/login` | Start SuperGrok OAuth login. |
| `auth/supergrok/status` | Read SuperGrok OAuth status. |
| `auth/supergrok/logout` | Clear SuperGrok OAuth credentials. |

Sessions, threads, and turns:

| Method | Purpose |
| --- | --- |
| `thread/start` | Create a desktop thread/session. |
| `thread/list` | List desktop threads. |
| `thread/read` | Read a desktop thread with optional turns. |
| `turn/start` | Start a desktop turn from rich text input. |
| `turn/steer` | Add user input to an active desktop turn. |
| `turn/interrupt` | Interrupt an active desktop turn. |
| `session/get` | Read policy mode and pending plan-exit state. |
| `session/set_mode` | Set the live policy mode. |
| `session/exit_plan` | Resolve a pending plan-exit request. |
| `session/resolve_approval` | Resolve a pending tool approval request. |
| `session/resolve_user_input` | Resolve a pending model-requested user input request. |

Tools, commands, files, agents, and tasks:

| Method | Purpose |
| --- | --- |
| `tools/list` | List runtime tool specs. |
| `tools/call` | Directly call allowed workflow tools. |
| `commands/list` | List configured slash commands. |
| `commands/expand` | Expand a command to a model prompt and context blocks. |
| `commands/run` | Expand a command and start a turn. |
| `fs/readFile` | Read an absolute host file as base64. |
| `fs/readDirectory` | List direct children of an absolute host directory. |
| `command/exec` | Run a non-PTY command subject to policy checks. |
| `agents/list` | List subagent definitions visible to the runtime. |
| `tasks/submit` | Submit a background task. |
| `tasks/list` | List task handles. |
| `tasks/get` | Read task handle plus logs. |
| `tasks/cancel` | Cancel a task. |
| `tasks/subscribe` | Return supported task event kinds. |

Teams and panes:

| Method | Purpose |
| --- | --- |
| `team/start` | Start an agent team. |
| `team/list` | List active/persisted teams. |
| `team/read` | Read a team and mailbox messages. |
| `team/member/start` | Add a teammate. |
| `team/member/message` | Send a direct teammate message. |
| `team/member/interrupt` | Interrupt a teammate. |
| `team/member/focus` | Validate and acknowledge focused teammate. |
| `team/cleanup` | Cleanup team state. |
| `team/pane/focus` | Unsupported in headless app-server clients. |
| `team/pane/cleanup` | Unsupported in headless app-server clients. |

Review, hunks, workflow imports, media, and memory:

| Method | Purpose |
| --- | --- |
| `turn/subagentTraces/list` | List subagent traces for a turn. |
| `turn/subagentTrace/read` | Read paged subagent trace deltas. |
| `plan/review/read` | Read a plan review. |
| `plan/review/comment` | Add a review comment and steer the turn. |
| `plan/review/rewrite` | Request a plan rewrite and steer the turn. |
| `plan/review/approve` | Approve a plan review. |
| `plan/review/reject` | Reject a plan review. |
| `hunk/list` | List recorded hunks, optionally by turn/review. |
| `hunk/read` | Read a paged hunk diff. |
| `hunk/rollback` | Confirm and apply a hunk reverse patch. |
| `workflow/scan` | Scan workflow imports. |
| `workflow/preview` | Preview workflow import items. |
| `workflow/enable` | Enable a workflow import. |
| `workflow/ignore` | Ignore a workflow import. |
| `workflow/refresh` | Re-scan and detect stale enabled imports. |
| `workflow/remove` | Remove an enabled workflow import decision. |
| `marketplaces/list` | List plugin marketplace descriptors. |
| `marketplaces/install_default` | Install one or all baked-in marketplace descriptors. |
| `marketplaces/add` | Add a local plugin marketplace descriptor. |
| `marketplaces/refresh` | Read and normalize a marketplace catalog. |
| `marketplaces/search` | Search de-duplicated marketplace plugins. |
| `marketplaces/plugin` | Read one marketplace plugin variant. |
| `plugins/preview_install` | Preview plugin install metadata and risk hints. |
| `plugins/install` | Record an installed marketplace plugin variant. |
| `plugins/list_installed` | List installed marketplace plugin variants. |
| `plugins/uninstall` | Remove an installed marketplace plugin variant record. |
| `media/list` | List media artifacts. |
| `media/read` | Read artifact bytes as base64. |
| `media/thumbnail` | Read an artifact preview. |
| `media/delete` | Delete an artifact. |
| `media/attachToTurn` | Convert an artifact to a turn attachment/image. |
| `memory/list` | List memory records. |
| `memory/read` | Read one memory. |
| `memory/save` | Save a memory. |
| `memory/update` | Update a memory. |
| `memory/delete` | Delete a memory. |
| `memory/query` | Search memories. |
| `memory/provider/list` | List embedding providers and selected provider. |
| `memory/provider/set` | Persist the embedding provider/model. |
| `memory/recall/preview` | Preview recall citations/results for a turn. |

## Detailed Method Reference

### `initialize`

Purpose: Perform the desktop startup handshake.

Request:

```json
{}
```

Response:

```json
{
  "provider": "openai",
  "model": "gpt-5.5",
  "cwd": "/Users/pz/w/gode"
}
```

Behavior:

- Reads the runtime default provider/model.
- Uses the process current directory for `cwd` when available.

Errors:

- None expected from the current handler beyond serialization/runtime failure.

### `providers/list`

Purpose: Discover provider auth state, models, and capabilities.

Request:

```json
{}
```

Response:

```json
{
  "active_provider": "openai",
  "active_model": "gpt-5.5",
  "active_reasoning": "high",
  "providers": [
    {
      "id": "openai",
      "name": "OpenAI",
      "description": "OpenAI Responses API",
      "auth_type": "api_key",
      "auth_label": "OPENAI_API_KEY",
      "authenticated": true,
      "auth_detail": null,
      "recommended": true,
      "sort_order": 0,
      "capabilities": {},
      "models": []
    }
  ]
}
```

Behavior:

- Providers are sorted by `sortOrder`, then name.
- OAuth providers report `authenticated` by checking the relevant token store.
- Model listing failures for an individual provider are treated as an empty
  model list.

### `providers/configure`

Purpose: Persist an API key for a registered API-key provider.

Request:

```json
{
  "provider": "opencode",
  "api_key": "sk-..."
}
```

Response:

```json
{
  "provider": "opencode",
  "authenticated": true
}
```

Behavior:

- Requires the provider to be registered in the runtime inference registry.
- Writes the key to the user config only when the app-server was created with
  user-config persistence enabled.

Errors:

- Empty or unknown providers return code `-32602`.
- Empty API keys return code `-32602`.
- Disabled user-config persistence returns code `-32000`.

### `providers/select`

Purpose: Select the active provider, model, and optional reasoning effort.

Request:

```json
{
  "provider": "openai",
  "model": "gpt-5.5",
  "reasoning": "high"
}
```

Response:

```json
{
  "provider": "openai",
  "model": "gpt-5.5",
  "reasoning": "high"
}
```

Behavior:

- Calls the runtime provider selector.
- Persists defaults only when the app-server was created with user-config
  persistence enabled.

Errors:

- Runtime provider/model validation errors return code `-32000` with
  `data.details`.

### `settings/get`

Purpose: Read runtime settings that app-server clients commonly expose.

Request:

```json
{}
```

Response:

```json
{
  "web_search": { "mode": "cached" },
  "default_mode": "default"
}
```

Notes:

- `web_search.mode` is one of `disabled`, `cached`, or `live`.
- `default_mode` is a `PolicyMode` value from `roder-api`.

### `settings/set_web_search`

Purpose: Change hosted web-search mode.

Request:

```json
{
  "mode": "live"
}
```

Response:

```json
{
  "web_search": { "mode": "live" }
}
```

Behavior:

- Updates runtime state immediately.
- Persists `disabled`, `codex`, or `live` to config when persistence is
  enabled. `codex` is the persisted config value for cached hosted search.

### `settings/set_default_mode`

Purpose: Change the default policy mode.

Request:

```json
{
  "mode": "plan"
}
```

Response:

```json
{
  "default_mode": "plan"
}
```

Behavior:

- Calls runtime policy-mode update with reason `settings default mode`.
- Persists a config value only when user-config persistence is enabled.

### `auth/codex/*`, `auth/supergrok/*`

Purpose: Manage provider OAuth credentials for Codex and SuperGrok.

Request:

```json
{}
```

Response:

```json
{
  "signed_in": true,
  "account_id": "acct_123"
}
```

Behavior:

- `login` runs the provider login flow and returns `signedIn: true`.
- `status` returns whether tokens are present.
- `logout` clears tokens and returns `signedIn: false`.

Errors:

- Token-store or login-flow errors return code `-32000` with `data.details`.

### `thread/start`

Purpose: Create a desktop thread backed by a Roder session.

Request:

```json
{
  "model": "gpt-5.5",
  "modelProvider": "openai",
  "cwd": "/Users/pz/w/gode",
  "ephemeral": false
}
```

Response:

```json
{
  "thread": {
    "id": "thread-123",
    "sessionId": "thread-123",
    "preview": "Untitled thread",
    "modelProvider": "openai",
    "createdAt": 1770000000,
    "updatedAt": 1770000000,
    "status": { "type": "idle" },
    "cwd": "/Users/pz/w/gode"
  },
  "model": "gpt-5.5",
  "modelProvider": "openai",
  "cwd": "/Users/pz/w/gode"
}
```

Behavior:

- Creates a persisted runtime session with optional provider/model/workspace.
- Stores the selected provider/model for later `turn/start` overrides.
- Emits `thread/started`.
- `ephemeral` is accepted by the DTO but is not currently used by the handler.

### `thread/list`

Purpose: Bootstrap or refresh a desktop sidebar.

Request:

```json
{
  "limit": 100
}
```

Response:

```json
{
  "data": [
    {
      "id": "thread-123",
      "sessionId": "thread-123",
      "preview": "Fix tests",
      "modelProvider": "openai",
      "createdAt": 1770000000,
      "updatedAt": 1770000100,
      "status": { "type": "idle" },
      "cwd": "/Users/pz/w/gode",
      "name": "Fix tests"
    }
  ],
  "nextCursor": null,
  "backwardsCursor": null
}
```

Behavior:

- Lists persisted runtime sessions sorted by newest `updatedAt` first.
- Applies `limit` when supplied.
- Merges in desktop threads that are in memory but not in persisted sessions.
- Cursor fields are currently always null.

### `thread/read`

Purpose: Read one desktop thread and optionally include turns/items.

Request:

```json
{
  "threadId": "thread-123",
  "includeTurns": true
}
```

Response:

```json
{
  "thread": {
    "id": "thread-123",
    "sessionId": "thread-123",
    "preview": "Fix tests",
    "modelProvider": "openai",
    "createdAt": 1770000000,
    "updatedAt": 1770000100,
    "status": { "type": "idle" },
    "cwd": "/Users/pz/w/gode",
    "turns": [
      {
        "id": "turn-123",
        "items": [],
        "itemsView": "default",
        "status": "completed"
      }
    ]
  }
}
```

Behavior:

- Reads a persisted session snapshot first.
- Falls back to persisted session metadata and then in-memory desktop threads.
- Returns `{"thread": null}` when the thread is unknown.

### `turn/start`

Purpose: Start a desktop turn on a thread.

Request:

```json
{
  "threadId": "thread-123",
  "input": [
    { "type": "text", "text": "inspect this repo" }
  ]
}
```

Response:

```json
{
  "turnId": "turn-123"
}
```

Behavior:

- Concatenates text input blocks with newlines.
- Uses `prompt` as a transition fallback only when text input is empty.
- Uses the thread's selected provider/model when known.
- Starts a runtime turn and records the active turn id for optional
  `turn/interrupt`.

Notifications:

- `turn/started`
- `thread/status/changed` with status `running`
- zero or more `item/agentMessage/delta`, `item/started`, and `item/completed`
- terminal `turn/completed`
- `thread/status/changed` with status `idle`

### `turn/steer`

Purpose: Send additional user input to an active desktop turn.

Request:

```json
{
  "threadId": "thread-123",
  "expectedTurnId": "turn-123",
  "input": [
    { "type": "text", "text": "also check the app-server tests" }
  ]
}
```

Response:

```json
{
  "turnId": "turn-123"
}
```

Behavior:

- Requires `expectedTurnId`.
- Converts rich text input using the same logic as `turn/start`.
- Calls runtime steering for the supplied turn id.

### `turn/interrupt`

Purpose: Interrupt a desktop turn.

Request:

```json
{
  "threadId": "thread-123"
}
```

Response:

```json
{
  "turnId": "turn-123"
}
```

Behavior:

- Uses `turnId` when supplied.
- Otherwise looks up the active turn recorded by `turn/start`.
- Removes the active-turn record after interrupting.

Errors:

- If no `turnId` is supplied and no active turn is known, returns code
  `-32602` with message `no active turn for thread ...`.

### `session/get`

Purpose: Read current policy mode and any pending plan-exit request.

Request:

```json
{}
```

Response:

```json
{
  "mode": "plan",
  "pending_plan_exit": {
    "thread_id": "thread-123",
    "turn_id": "turn-123",
    "request_id": "request-123",
    "target_mode": "default",
    "plan_summary": "Implement the test first.",
    "requested_at": "2026-05-18T12:00:00Z",
    "expires_at": null
  }
}
```

### `session/set_mode`

Purpose: Set the live policy mode.

Request:

```json
{
  "mode": "accept_edits",
  "reason": "desktop toggle"
}
```

Response:

```json
{
  "mode": "accept_edits"
}
```

### `session/exit_plan`

Purpose: Approve or reject a pending plan-mode exit.

Request:

```json
{
  "request_id": "request-123",
  "approved": true
}
```

Response:

```json
{
  "resolved": true,
  "mode": "default"
}
```

### `session/resolve_approval`

Purpose: Resolve a pending tool approval.

Request:

```json
{
  "approval_id": "approval-123",
  "approved": true
}
```

Response:

```json
{
  "resolved": true
}
```

### `session/resolve_user_input`

Purpose: Resolve a pending `request_user_input` tool request.

Request:

```json
{
  "request_id": "input-123",
  "answers": {
    "choice": "continue"
  }
}
```

Response:

```json
{
  "resolved": true
}
```

### `fs/readFile`

Purpose: Read a file from the host filesystem.

Request:

```json
{
  "path": "/Users/pz/w/gode/README.md"
}
```

Response:

```json
{
  "dataBase64": "IyBSb2Rlcgo="
}
```

Errors:

- Relative paths return code `-32602` and message `path must be absolute`.
- Filesystem read errors return code `-32000` with `data.details`.

### `fs/readDirectory`

Purpose: List direct children of an absolute host directory.

Request:

```json
{
  "path": "/Users/pz/w/gode/docs"
}
```

Response:

```json
{
  "entries": [
    { "fileName": "api.md", "isDirectory": false, "isFile": true }
  ]
}
```

Behavior:

- Entries are sorted by `fileName`.
- Only direct children are returned.

### `command/exec`

Purpose: Run a one-off non-PTY command under the current policy mode.

Request:

```json
{
  "command": ["cargo", "test", "-p", "roder-app-server"],
  "processId": "process-123",
  "cwd": "/Users/pz/w/gode",
  "env": {
    "RUST_LOG": "info",
    "NO_COLOR": null
  },
  "timeoutMs": 30000,
  "outputBytesCap": 1048576,
  "streamStdoutStderr": true
}
```

Response when `streamStdoutStderr` is false:

```json
{
  "exitCode": 0,
  "stdout": "ok\n",
  "stderr": ""
}
```

Response when `streamStdoutStderr` is true:

```json
{
  "exitCode": 0,
  "stdout": "",
  "stderr": ""
}
```

Behavior:

- Requires `command` to be non-empty.
- `cwd` must be absolute when supplied.
- Default timeout is 30000 ms unless `disableTimeout` is true.
- Default output cap is 1048576 bytes unless `disableOutputCap` is true.
- When streaming is enabled, `processId` is required and stdout/stderr are sent
  as `command/exec/outputDelta` notifications.
- Command execution is checked by the runtime policy gate as a `shell` tool.

Unsupported:

- `tty`, `streamStdin`, and resize via `size` return code `-32004` with
  `data.kind: "unsupported"`.

Validation:

- `disableTimeout` cannot be combined with `timeoutMs`.
- `disableOutputCap` cannot be combined with `outputBytesCap`.

### `tools/list`

Purpose: List tools available to the runtime.

Request:

```json
{}
```

Response:

```json
{
  "tools": [
    {
      "name": "exec_command",
      "description": "Run a command",
      "input_schema": {}
    }
  ]
}
```

### `tools/call`

Purpose: Directly call selected workflow tools.

Request:

```json
{
  "thread_id": "thread-123",
  "tool_name": "get_goal",
  "arguments": {}
}
```

Response:

```json
{
  "text": "",
  "data": {},
  "is_error": false
}
```

Behavior:

- Only `get_goal` and `create_goal` can be called directly.
- Other tool names return code `-32602`.

### `commands/list`

Purpose: List available slash commands.

Request:

```json
{}
```

Response:

```json
{
  "commands": [
    {
      "name": "test",
      "description": "Run tests",
      "argument_hint": "[package]",
      "source": "builtin",
      "model": null,
      "agent": null,
      "has_shell_includes": false,
      "has_url_includes": false
    }
  ]
}
```

### `commands/expand`

Purpose: Expand a command into prompt text and context blocks without running
it.

Request:

```json
{
  "name": "test",
  "arguments": "roder-app-server",
  "workspace": "/Users/pz/w/gode"
}
```

Response:

```json
{
  "command": {
    "name": "test",
    "description": "Run tests",
    "argument_hint": "[package]",
    "source": "builtin",
    "model": null,
    "agent": null,
    "has_shell_includes": false,
    "has_url_includes": false
  },
  "message": "Run tests for roder-app-server",
  "context_blocks": [],
  "allowed_tools": [],
  "model": null,
  "agent": null
}
```

Errors:

- Unknown commands return code `-32602`.
- Disabled command configuration returns code `-32000`.
- Missing workspace resolution returns code `-32000`.

### `commands/run`

Purpose: Expand a command and start a turn with the expanded prompt.

Request:

```json
{
  "thread_id": "thread-123",
  "name": "test",
  "arguments": "roder-app-server",
  "workspace": "/Users/pz/w/gode"
}
```

Response:

```json
{
  "turn_id": "turn-123",
  "expanded": {
    "command": {
      "name": "test",
      "description": "Run tests",
      "argument_hint": "[package]",
      "source": "builtin",
      "model": null,
      "agent": null,
      "has_shell_includes": false,
      "has_url_includes": false
    },
    "message": "Run tests for roder-app-server",
    "context_blocks": [],
    "allowed_tools": [],
    "model": null,
    "agent": null
  }
}
```

### `tasks/submit`

Purpose: Submit a background task to a registered executor.

Request:

```json
{
  "executor_id": "task",
  "input": { "prompt": "summarize docs" },
  "thread_id": "thread-123",
  "turn_id": "turn-123",
  "workspace": "/Users/pz/w/gode"
}
```

Response:

```json
{
  "task": {
    "task_id": "task-123",
    "executor_id": "task",
    "status": "running"
  }
}
```

Behavior:

- Uses explicit `workspace`, then runtime workspace, then process cwd.
- If a remote runner is selected, creates a runner session for the task.
- Errors if the selected remote runner provider is not installed.

### `tasks/list`, `tasks/get`, `tasks/cancel`, `tasks/subscribe`

Purpose: Inspect and manage background tasks.

Examples:

```json
{
  "method": "tasks/get",
  "params": { "task_id": "task-123" }
}
```

```json
{
  "task": {
    "task_id": "task-123",
    "executor_id": "task",
    "status": "completed"
  },
  "logs": [
    { "stream": "stdout", "chunk": "done\n", "timestamp": "2026-05-18T12:00:00Z" }
  ],
  "dropped_bytes": 0
}
```

Errors:

- Unknown task ids return code `-32602`.

`tasks/subscribe` currently returns:

```json
{
  "subscribed": true,
  "event_kinds": [
    "task.started",
    "task.output",
    "task.completed",
    "task.failed",
    "task.cancelled"
  ]
}
```

### `runners/list`

Purpose: Discover remote runner providers and current runner selection.

Request:

```json
{}
```

Response:

```json
{
  "active": null,
  "providers": [
    {
      "provider_id": "docker",
      "capabilities": {}
    }
  ]
}
```

### `runners/select`

Purpose: Select a remote runner destination.

Request:

```json
{
  "destination_id": "local-docker",
  "provider_id": "docker",
  "config": {},
  "manifest": {}
}
```

Response:

```json
{
  "active": {
    "destination_id": "local-docker",
    "provider_id": "docker",
    "state": "configured",
    "session_id": null
  }
}
```

Behavior:

- Defaults `provider_id` to `destination_id` when omitted.
- Validates the destination through the selected provider.
- Stores the runner destination on runtime state.

Errors:

- Unknown provider returns code `-32602`.
- Provider validation errors return code `-32602` with `data.details`.

### Runner utility methods

Purpose: Read or clear runner state.

Methods:

- `runners/session` returns the active runner destination.
- `runners/snapshot` currently returns `{ "snapshot": null }`.
- `runners/delete` clears the selected destination and returns
  `{ "deleted": true }`.
- `runners/ports` currently returns `{ "ports": [] }`.

### `team/start`

Purpose: Create an agent team.

Request:

```json
{
  "leadThreadId": "thread-123",
  "displayMode": "in_process",
  "members": [
    { "name": "Reviewer", "modelProvider": "openai", "model": "gpt-5.5" }
  ]
}
```

Response:

```json
{
  "team": {
    "id": "team-123",
    "leadThreadId": "thread-123",
    "displayMode": "in_process",
    "members": [],
    "tasks": []
  }
}
```

Behavior:

- Defaults display mode through `AgentTeamDisplayMode::default()` when omitted.
- Emits `team/started`.

### Team member methods

Purpose: Manage team state and route messages to teammates.

Examples:

```json
{
  "method": "team/member/message",
  "params": {
    "teamId": "team-123",
    "memberId": "member-123",
    "text": "review this patch",
    "expectedTurnId": null
  }
}
```

```json
{
  "turnId": "turn-456"
}
```

Behavior:

- `team/list` applies optional `limit` and currently returns `nextCursor: null`.
- `team/read` returns `{ "team": null, "messages": [] }` for unknown teams.
- `team/member/start` returns the newly appended member.
- `team/member/interrupt` returns whether a member turn was interrupted.
- `team/member/focus` validates the team and member and echoes
  `focusedMemberId`.
- `team/cleanup` delegates to runtime cleanup and honors `force`.
- `team/pane/focus` and `team/pane/cleanup` return method-not-found style code
  `-32601` with `data.supportedAlternative: "team/member/focus"` because split
  panes are TUI-local.

### Subagent traces

Purpose: Read subagent trace summaries and paged trace deltas from persisted
session events.

Examples:

```json
{
  "method": "turn/subagentTraces/list",
  "params": {
    "threadId": "thread-123",
    "turnId": "turn-123"
  }
}
```

```json
{
  "method": "turn/subagentTrace/read",
  "params": {
    "threadId": "thread-123",
    "traceId": "trace-123",
    "offset": 0,
    "limit": 100
  }
}
```

Behavior:

- Missing sessions return empty trace lists or empty event pages.
- Read defaults to `limit: 100`; limit is clamped to at least 1.
- `nextOffset` is present only when more events remain.

### Plan review methods

Purpose: Read and mutate plan review artifacts.

Examples:

```json
{
  "method": "plan/review/comment",
  "params": {
    "threadId": "thread-123",
    "reviewId": "review-123",
    "anchor": { "kind": "summary" },
    "body": "Please include tests."
  }
}
```

Behavior:

- `plan/review/read` returns `{ "review": null }` for unknown reviews.
- `comment` and `rewrite` emit runtime events and steer the review's turn with
  a synthesized message.
- `approve` and `reject` emit runtime events.
- Unknown review ids for mutating methods return code `-32602`.

### Hunk methods

Purpose: List, read, and rollback recorded file hunks.

Examples:

```json
{
  "method": "hunk/list",
  "params": {
    "threadId": "thread-123",
    "turnId": "turn-123",
    "reviewId": "review-123"
  }
}
```

```json
{
  "method": "hunk/rollback",
  "params": {
    "threadId": "thread-123",
    "hunkId": "hunk-123",
    "confirmed": true
  }
}
```

Behavior:

- `hunk/list` can filter by `turnId` and `reviewId`.
- `hunk/read` pages diff output with default limit 100 and minimum limit 1.
- `hunk/rollback` first emits `hunk/rollbackRequested`, then emits
  `hunk/rollbackCompleted`.
- Rollback requires `confirmed: true` and a recorded reverse patch.
- A successful rollback returns `{ "rolledBack": true }`; failures return
  `{ "rolledBack": false, "error": "..." }`.

### Workflow import methods

Purpose: Scan, preview, enable, ignore, refresh, and remove workflow imports for
AGENTS.md, skills, MCP, hooks, commands, and plugin-like artifacts.

Examples:

```json
{
  "method": "workflow/scan",
  "params": {
    "workspace": "/Users/pz/w/gode",
    "includeUser": true
  }
}
```

```json
{
  "method": "workflow/enable",
  "params": {
    "workspace": "/Users/pz/w/gode",
    "itemId": "skill:roder-app-server-docs",
    "approveSideEffects": true
  }
}
```

Behavior:

- When `workspace` is omitted, the handler uses runtime workspace then process
  cwd.
- `includeUser: true` also scans `~/.roder` and `~/.agents`.
- `enable`, `ignore`, and `remove` persist decisions to
  `~/.roder/workflow-imports.json` unless overridden by
  `RODER_WORKFLOW_IMPORTS_PATH`.
- Enabling an item that requires approval without `approveSideEffects` returns
  code `-32040` with `itemId`, `source`, and `risk` in `data`.

### Plugin marketplace methods

Purpose: Manage Claude, Cursor, Codex, and local plugin marketplace metadata,
search de-duplicated plugin entries, preview plugin installs, and record local
installed plugin variants.

Marketplace state is persisted under `~/.roder/marketplaces.json` unless
`RODER_MARKETPLACES_PATH` is set. Plugin cache markers are written under
`~/.roder/plugins/cache` unless `RODER_MARKETPLACE_CACHE_DIR` is set.

#### `marketplaces/list`

Purpose: List configured marketplace descriptors, including baked-in defaults.

Request:

```json
{}
```

Response:

```json
{
  "marketplaces": [
    {
      "id": "cursor-plugins",
      "kind": "cursor",
      "displayName": "Cursor Marketplace",
      "source": {
        "kind": "github",
        "repo": "cursor/plugins",
        "catalogPath": ".cursor-plugin/marketplace.json"
      },
      "homepage": "https://cursor.com/en-US/marketplace",
      "isDefault": true,
      "enabled": true,
      "state": "bakedIn"
    }
  ]
}
```

Behavior:

- Always merges the baked-in Claude, Cursor, and Codex marketplace descriptors
  into the loaded store.
- Does not refresh remote or local catalogs.
- `state` is one of `bakedIn`, `installed`, `refreshed`, `disabled`, or
  `removedByUser`.

Errors:

- Returns code `-32000` if the marketplace store cannot be read or parsed.

#### `marketplaces/install_default`

Purpose: Register one or more baked-in default marketplace descriptors.

Request:

```json
{
  "selection": "all"
}
```

Response:

```json
{
  "marketplaces": [
    {
      "id": "claude-plugins-official",
      "kind": "claude",
      "displayName": "Claude Plugins Official",
      "source": {
        "kind": "github",
        "repo": "anthropics/claude-plugins-official",
        "catalogPath": ".claude-plugin/marketplace.json"
      },
      "isDefault": true,
      "enabled": true,
      "state": "installed"
    }
  ]
}
```

Behavior:

- `selection` accepts `none`, `anthropic`, `cursor`, `codex`, and `all`.
- This registers marketplace descriptors only; it does not install every plugin
  in those catalogs.
- Re-running the method is idempotent and updates matching descriptors instead
  of duplicating them.
- The current implementation records default GitHub-backed descriptors. It does
  not fetch their remote catalogs during default installation.

Errors:

- Invalid `selection` values fail JSON deserialization and return code
  `-32602`.
- Store write failures return code `-32000`.

#### `marketplaces/add`

Purpose: Add a custom local marketplace descriptor.

Request:

```json
{
  "id": "local-cursor",
  "kind": "cursor",
  "displayName": "Local Cursor",
  "localPath": "/Users/pz/plugins/cursor"
}
```

Response:

```json
{
  "marketplace": {
    "id": "local-cursor",
    "kind": "cursor",
    "displayName": "Local Cursor",
    "source": {
      "kind": "localPath",
      "path": "/Users/pz/plugins/cursor"
    },
    "isDefault": false,
    "enabled": true,
    "state": "installed"
  }
}
```

Behavior:

- `id` must be a lowercase slug starting and ending with an ASCII letter or
  number. Interior `-` and `.` are allowed.
- `kind` is one of `claude`, `cursor`, `codex`, `roder`, or `custom`.
- `localPath` must exist when the request is handled.
- Existing marketplace records with the same `id` are replaced.

Errors:

- Invalid ids or missing local paths return code `-32602`.
- Store read/write failures return code `-32000`.

#### `marketplaces/refresh`

Purpose: Read a marketplace catalog and return normalized plugin entries.

Request:

```json
{
  "marketplaceId": "local-cursor"
}
```

Response:

```json
{
  "marketplace": {
    "id": "local-cursor",
    "kind": "cursor",
    "displayName": "Local Cursor",
    "source": {
      "kind": "localPath",
      "path": "/Users/pz/plugins/cursor"
    },
    "enabled": true,
    "state": "refreshed",
    "lastRefreshedAt": "2026-05-18T18:00:00Z",
    "contentHash": "a275f0a3080931b1..."
  },
  "plugins": [
    {
      "marketplaceId": "local-cursor",
      "pluginId": "repo-tools",
      "displayName": "Repo Tools",
      "kind": "cursor",
      "source": {
        "kind": "marketplacePath",
        "marketplace_id": "local-cursor",
        "path": "Repo Tools"
      },
      "identityKey": {
        "canonicalSlug": "repo-tools",
        "normalizedName": "repo-tools"
      },
      "tags": ["repo"],
      "componentHints": {
        "skills": true,
        "commands": false,
        "agents": false,
        "mcpServers": false,
        "hooks": false,
        "apps": false,
        "lspServers": false,
        "rules": false,
        "assets": false
      },
      "capabilityHints": [],
      "risk": "passive",
      "rawManifest": {
        "id": "repo-tools",
        "name": "Repo Tools"
      }
    }
  ]
}
```

Behavior:

- Local Claude marketplaces read `.claude-plugin/marketplace.json`.
- Local Cursor marketplaces read `.cursor-plugin/marketplace.json`.
- Local Codex marketplaces scan plugin directories for
  `.codex-plugin/plugin.json`.
- The marketplace record is updated to `state: "refreshed"` with
  `lastRefreshedAt` and `contentHash`.
- Current refresh support is local-path based. GitHub, git URL, and HTTP JSON
  descriptors may be listed, but refresh of those sources currently returns an
  internal error instead of fetching the network.

Errors:

- Unknown marketplace ids, unsupported source resolution, missing catalog files,
  invalid JSON, and normalization failures return code `-32000`.

#### `marketplaces/search`

Purpose: Search installed/refreshed local marketplaces and return de-duplicated
plugin rows.

Request:

```json
{
  "query": "repo"
}
```

Response:

```json
{
  "plugins": [
    {
      "identityKey": {
        "canonicalSlug": "repo-tools",
        "normalizedName": "repo-tools"
      },
      "displayName": "Repo Tools",
      "description": "Repository helper skills",
      "variants": [
        {
          "marketplaceId": "local-cursor",
          "pluginId": "repo-tools",
          "kind": "cursor",
          "source": {
            "kind": "marketplacePath",
            "marketplace_id": "local-cursor",
            "path": "Repo Tools"
          },
          "risk": "passive"
        }
      ],
      "installedVariants": []
    }
  ]
}
```

Behavior:

- Omit `query` or pass an empty string to return every searchable plugin.
- Search matches plugin id, display name, description, and tags.
- Results are de-duplicated by strong identity signals: repository, homepage
  plus normalized name, or provider-local slug.
- Baked-in default descriptors are skipped until installed/refreshed and
  resolvable locally. Remote descriptor fetch is not implicit.

Errors:

- Store read failures, catalog parse failures, and normalization failures return
  code `-32000`.

#### `marketplaces/plugin`

Purpose: Return one normalized plugin variant by marketplace id and plugin id.

Request:

```json
{
  "marketplaceId": "local-cursor",
  "pluginId": "repo-tools"
}
```

Response:

```json
{
  "plugin": {
    "marketplaceId": "local-cursor",
    "pluginId": "repo-tools",
    "displayName": "Repo Tools",
    "kind": "cursor",
    "source": {
      "kind": "marketplacePath",
      "marketplace_id": "local-cursor",
      "path": "Repo Tools"
    },
    "identityKey": {
      "canonicalSlug": "repo-tools",
      "normalizedName": "repo-tools"
    },
    "tags": [],
    "componentHints": {
      "skills": true,
      "commands": false,
      "agents": false,
      "mcpServers": false,
      "hooks": false,
      "apps": false,
      "lspServers": false,
      "rules": false,
      "assets": false
    },
    "capabilityHints": [],
    "risk": "passive",
    "rawManifest": {}
  }
}
```

Behavior:

- Returns `{ "plugin": null }` when the marketplace can be read but no matching
  plugin exists.

Errors:

- Store, catalog, and normalization failures return code `-32000`.

#### `plugins/preview_install`

Purpose: Return install metadata and risk/component hints for a plugin variant
without recording an install.

Request:

```json
{
  "marketplaceId": "local-cursor",
  "pluginId": "repo-tools"
}
```

Response:

```json
{
  "preview": {
    "marketplaceId": "local-cursor",
    "pluginId": "repo-tools",
    "displayName": "Repo Tools",
    "identityKey": {
      "canonicalSlug": "repo-tools",
      "normalizedName": "repo-tools"
    },
    "source": {
      "kind": "marketplacePath",
      "marketplace_id": "local-cursor",
      "path": "Repo Tools"
    },
    "componentHints": {
      "skills": true,
      "commands": false,
      "agents": false,
      "mcpServers": false,
      "hooks": false,
      "apps": false,
      "lspServers": false,
      "rules": false,
      "assets": false
    },
    "capabilityHints": [],
    "risk": "passive",
    "rawManifest": {}
  }
}
```

Behavior:

- Does not execute package scripts, hooks, MCP servers, apps, or other
  plugin-provided commands.
- Preview currently reflects normalized catalog metadata and raw manifest data;
  it does not activate workflow imports.

Errors:

- Missing plugins return code `-32004` with message `plugin not found`.
- Store, catalog, and normalization failures return code `-32000`.

#### `plugins/install`

Purpose: Record one installed marketplace plugin variant in the local plugin
cache.

Request:

```json
{
  "marketplaceId": "local-cursor",
  "pluginId": "repo-tools"
}
```

Response:

```json
{
  "plugin": {
    "marketplaceId": "local-cursor",
    "pluginId": "repo-tools",
    "identityKey": {
      "canonicalSlug": "repo-tools",
      "normalizedName": "repo-tools"
    },
    "variantKey": "local-cursor:repo-tools",
    "installPath": "/Users/pz/.roder/plugins/cache/local-cursor/repo-tools/a275f0a3080931b1",
    "contentHash": "a275f0a3080931b1...",
    "state": "installed",
    "installedAt": "2026-05-18T18:00:00Z"
  }
}
```

Behavior:

- Records the installed variant separately from workflow import decisions.
- Reinstalling the same `marketplaceId` and `pluginId` replaces the installed
  record for the same `variantKey`.
- Writes a cache marker under the configured marketplace cache dir.
- Does not execute plugin-provided code and does not enable plugin components.

Errors:

- Missing plugins return code `-32004` with message `plugin not found`.
- Store, catalog, cache, and normalization failures return code `-32000`.

#### `plugins/list_installed`

Purpose: List installed marketplace plugin variant records.

Request:

```json
{}
```

Response:

```json
{
  "plugins": [
    {
      "marketplaceId": "local-cursor",
      "pluginId": "repo-tools",
      "identityKey": {
        "canonicalSlug": "repo-tools",
        "normalizedName": "repo-tools"
      },
      "variantKey": "local-cursor:repo-tools",
      "installPath": "/Users/pz/.roder/plugins/cache/local-cursor/repo-tools/a275f0a3080931b1",
      "contentHash": "a275f0a3080931b1...",
      "state": "installed",
      "installedAt": "2026-05-18T18:00:00Z"
    }
  ]
}
```

Errors:

- Store read failures return code `-32000`.

#### `plugins/uninstall`

Purpose: Remove one installed plugin variant record.

Request:

```json
{
  "variantKey": "local-cursor:repo-tools"
}
```

Response:

```json
{
  "removed": true
}
```

Behavior:

- Removes only the installed plugin record from marketplace state.
- Does not delete unrelated user data.
- Returns `removed: false` when no matching `variantKey` exists.

Errors:

- Store read/write failures return code `-32000`.

### Media methods

Purpose: Manage terminal media artifacts and turn attachments.

Examples:

```json
{
  "method": "media/list",
  "params": {
    "threadId": "thread-123",
    "kind": "image"
  }
}
```

```json
{
  "method": "media/read",
  "params": {
    "artifactId": "artifact-123",
    "maxBytes": 1048576
  }
}
```

Behavior:

- `media/list` currently filters by `kind`; `threadId` is accepted by the DTO
  but not used by the handler.
- `media/read` returns artifact metadata plus `bytesBase64`.
- `media/thumbnail` returns a `MediaPreview`.
- `media/delete` emits `media/artifactDeleted` when deletion succeeds.
- `media/attachToTurn` returns a `MediaAttachment` and an `InputImage` only for
  image artifacts.
- The media store root comes from config, `RODER_MEDIA_ARTIFACT_DIR`, or the
  default media artifact directory. Default max read size is 10 MiB.

### Memory methods

Purpose: Manage and search memories through the registered memory store.

Examples:

```json
{
  "method": "memory/save",
  "params": {
    "scope": { "kind": "project", "workspace": "/Users/pz/w/gode" },
    "text": "Roder app-server uses JSON-RPC.",
    "metadata": { "source": "docs" }
  }
}
```

```json
{
  "method": "memory/query",
  "params": {
    "scope": { "kind": "project", "workspace": "/Users/pz/w/gode" },
    "text": "app-server JSON-RPC",
    "limit": 10,
    "includeGlobal": true
  }
}
```

Behavior:

- `memory/list` defaults `limit` to 50.
- `memory/query` defaults `limit` to 10.
- `memory/recall/preview` defaults `limit` to 5 and emits
  `memory/recallReady`.
- Save/update/delete/query/provider changes emit memory notifications.
- If no memory store is registered, memory methods return code `-32000` and
  message `No memory store is registered`.

### Memory provider methods

Purpose: Inspect and persist the embedding provider/model used by memory recall.

Examples:

```json
{
  "method": "memory/provider/set",
  "params": {
    "providerId": "openai",
    "model": "text-embedding-3-large"
  }
}
```

Behavior:

- `memory/provider/list` returns registered embedding providers and the selected
  provider/model from config, defaulting to OpenAI
  `text-embedding-3-large`.
- `memory/provider/set` writes config and emits `memory/providerChanged`.

## Streaming and Notifications

Subscribe through `LocalAppClient::subscribe_notifications()` for local clients
or the remote WebSocket notification stream for remote clients.

### Turn and item notifications

`thread/started`:

```json
{
  "thread": {
    "id": "thread-123",
    "sessionId": "thread-123",
    "preview": "Untitled thread",
    "modelProvider": "openai",
    "createdAt": 1770000000,
    "updatedAt": 1770000000,
    "status": { "type": "idle" },
    "cwd": "/Users/pz/w/gode"
  }
}
```

`turn/started`:

```json
{
  "threadId": "thread-123",
  "turn": {
    "id": "turn-123",
    "items": [],
    "itemsView": "default",
    "status": "inProgress",
    "startedAt": 1770000000
  }
}
```

`item/agentMessage/delta`:

```json
{
  "threadId": "thread-123",
  "turnId": "turn-123",
  "itemId": "turn-123-agent-final_answer",
  "delta": "Hello",
  "phase": "final_answer"
}
```

`item/started` and `item/completed` carry `threadId`, `turnId`, and a desktop
`item` object. Tool items use `type: "tool.<name>"` when the tool name is known.

`turn/completed` carries `threadId` and a terminal `turn` whose `status` is
`completed`, `failed`, or `interrupted`.

`thread/status/changed`:

```json
{
  "threadId": "thread-123",
  "status": { "type": "running", "activeFlags": [] }
}
```

Ordering:

- `turn/started` is emitted before terminal `turn/completed`.
- A running status notification is emitted when a turn starts.
- An idle status notification is emitted after completed, failed, or
  interrupted terminal turn notifications.

### Command output

`command/exec/outputDelta` is emitted only when `streamStdoutStderr` is true:

```json
{
  "processId": "process-123",
  "stream": "stdout",
  "deltaBase64": "b2sK",
  "capReached": false
}
```

### Team notifications

Team notifications include:

- `team/started`
- `team/member/started`
- `team/member/statusChanged`
- `team/member/messageDelta`
- `team/member/completed`
- `team/cleanupCompleted`

Example:

```json
{
  "teamId": "team-123",
  "memberId": "member-123",
  "turnId": "turn-456",
  "delta": "Reviewing"
}
```

### Advanced artifact notifications

The app-server forwards these event families as same-named JSON-RPC
notifications:

- Subagent traces: `turn/subagentTraceCreated`, `turn/subagentTraceDelta`,
  `turn/subagentTraceStatusChanged`, `turn/subagentTraceCompleted`,
  `turn/subagentTraceFailed`.
- Plan review: `plan/reviewCreated`, `plan/reviewStatusChanged`,
  `plan/reviewCommentAdded`, `plan/reviewRewritten`,
  `plan/reviewApproved`, `plan/reviewRejected`.
- Hunks: `hunk/recorded`, `hunk/rollbackRequested`,
  `hunk/rollbackCompleted`.
- Workflow imports: `workflow/importsDetected`, `workflow/importPreviewed`,
  `workflow/importEnabled`, `workflow/importDisabled`,
  `workflow/importStale`, `workflow/importFailed`.
- Media: `media/artifactCreated`, `media/artifactUpdated`,
  `media/artifactDeleted`, `media/previewReady`.
- Memory: `memory/saved`, `memory/updated`, `memory/deleted`,
  `memory/queried`, `memory/recallReady`, `memory/reembedQueued`,
  `memory/providerChanged`, `memory/observationRecorded`.

Payloads for these notifications are the corresponding `roder-api` event
structs serialized to JSON.

## Error Model

Common errors:

```json
{
  "code": -32601,
  "message": "Method not found"
}
```

```json
{
  "code": -32602,
  "message": "Invalid params: missing field `threadId`"
}
```

```json
{
  "code": -32000,
  "message": "provider error",
  "data": { "details": "provider error" }
}
```

Error conventions:

- `-32601`: unknown methods and unsupported split-pane methods in headless
  clients.
- `-32602`: JSON decoding failures, validation errors, unknown ids, relative
  paths, and invalid runner/team/hunk/memory references.
- `-32000`: runtime, filesystem, provider, config, task, command expansion,
  media, memory-store, and other internal errors. Most include
  `data.details`.
- `-32004`: command policy denial or unsupported `command/exec` mode.
- `-32040`: workflow import approval is required.

Cancellation and interruption:

- `turn/interrupt` calls the runtime interrupt path.
- `team/member/interrupt` interrupts only the selected member.
- `tasks/cancel` cancels a background task and returns `{ "cancelled": bool }`.

## Persistence and Contract Notes

- `thread/list` and `thread/read` use persisted sessions first and in-memory
  desktop threads as a fallback.
- `providers/select`, `settings/set_web_search`, and
  `settings/set_default_mode` persist only when the app-server instance enables
  user-config persistence.
- Workflow import decisions are persisted under `~/.roder/workflow-imports.json`
  unless `RODER_WORKFLOW_IMPORTS_PATH` is set.
- Media artifact storage is configured by `media.artifacts_dir`,
  `RODER_MEDIA_ARTIFACT_DIR`, or the default media directory.
- `media/list` accepts `threadId` but currently filters only by `kind`.
- `thread/start` accepts `ephemeral` but the handler currently does not use it.
- Cursor fields in `thread/list` and `team/list` are reserved and currently
  null.

## Integration Recipes

### Startup and Sidebar Bootstrap

1. Call `initialize`.
2. Call `providers/list` and `model/list`.
3. Call `settings/get`.
4. Call `thread/list` with a reasonable limit.
5. Subscribe to notifications before starting or attaching to active turns.

### Create a Thread and Run a Turn

1. Call `thread/start` with `model`, `modelProvider`, and `cwd`.
2. Wait for `thread/started` or use the returned `thread`.
3. Call `turn/start` with rich text `input`.
4. Consume notifications until matching `turn/completed`.
5. Treat `thread/status/changed` `idle` as the sidebar busy-state clear.

### Resume a Thread

1. Call `thread/read` with `includeTurns: true`.
2. Render `thread.turns[].items`.
3. Subscribe to notifications.
4. Use `turn/start` for a new turn or `turn/steer` only when an active turn is
   known.

### Stop Work

1. Call `turn/interrupt` with `threadId`; include `turnId` when the client has
   it.
2. For teammate work, call `team/member/interrupt`.
3. For background tasks, call `tasks/cancel`.

### Run a Command with Streaming Output

1. Generate a client-side `processId`.
2. Call `command/exec` with `streamStdoutStderr: true`, `processId`, and an
   absolute `cwd`.
3. Append decoded `command/exec/outputDelta.deltaBase64` chunks by stream.
4. Use `capReached` to mark truncated output.
5. Use the final `exitCode` response as process completion.

### Provider Login and Selection

1. Call `providers/list`.
2. If the desired Codex provider has `authType: "oauth"` and
   `authenticated: false`, call `auth/codex/login`.
3. Call `providers/list` again or `auth/codex/status`.
4. Call `providers/select` with provider, model, and optional reasoning.

### Memories

1. Call `memory/provider/list` to show available embedding providers.
2. Optionally call `memory/provider/set`.
3. Use `memory/save` or `memory/update` for durable facts.
4. Use `memory/query` for search.
5. Use `memory/recall/preview` when preparing citations for a specific
   thread/turn.

## Maintenance Checklist

When changing the app-server surface, update this document after checking:

- `AppServer::handle_request` method registration.
- DTOs in `crates/roder-protocol/src/lib.rs`.
- Handler behavior in `crates/roder-app-server/src/server.rs`,
  `command.rs`, `fs.rs`, and `remote.rs`.
- Notification mapping in `crates/roder-app-server/src/notifications.rs`.
- E2E tests in `crates/roder-app-server/tests/e2e.rs`.
- Auth/config persistence behavior and environment variables.
- Removed methods and explicitly unsupported methods.
