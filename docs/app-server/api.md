# Roder App-Server API

This document is the canonical integrator-facing reference for the Roder
app-server API. It describes the JSON-RPC methods implemented by
`crates/roder-app-server`, the shared wire DTOs in `crates/roder-protocol`, and
the notification stream emitted to app, TUI, SDK, or sibling clients.

> Maintenance note: update this document with the `roder-app-server-docs` skill
> whenever app-server methods, request/response types, events, auth/config
> behavior, provider/model behavior, or thread semantics change.

## Overview

The app-server is a JSON-RPC 2.0 control plane for Roder runtime state. Clients
use it to:

- initialize against the current runtime, provider, model, workspace, and
  settings.
- create or resume threads.
- start, steer, interrupt, and observe turns.
- list/select providers, models, runners, tools, agents, commands, skills,
  memories, media artifacts, workflow imports, plan reviews, hunks,
  automations, eval reports, retrieval diagnostics, search indexes, code
  indexes, and background tasks.
- receive notifications for turn lifecycle, streamed assistant output, tool
  lifecycle, teams, workflow imports, media, memory, plan review, hunk,
  discovery, retrieval, skill, search-index, code-index, and automation events.

The source of truth for method registration is
`AppServer::handle_request` in `crates/roder-app-server/src/server.rs`.
The versioned public method manifest is checked in at
`schemas/app-server/roder-app-server.v1.json`; the JSON Schema wrapper is
`schemas/app-server/methods.schema.json`. SDK generators and integrators should
prefer those files over scraping this prose.

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
Sec-WebSocket-Protocol: roder.remote.v1, bearer.<token>
```

Provider auth is provider-specific:

- `auth/codex/login`, `auth/codex/status`, and `auth/codex/logout` manage the
  Codex OAuth token store through `roder-codex-auth`.
- `providers/list` reports each provider's `authType`, `authLabel`,
  `authenticated`, and optional `authDetail`.
- API-key providers rely on environment/config outside this app-server method
  surface.

Config persistence is opt-in on the `AppServer` instance. When enabled,
`providers/select`, `settings/set_web_search`, `settings/set_shell`,
`settings/set_default_mode`, and `settings/set_file_backed_dynamic_context`
write the selected defaults to
`~/.roder/config.toml`.

## Core Concepts

`thread` is the persisted runtime container and the top-level client-visible
thread object. It is shaped as:

```json
{
  "id": "thread-123",
  "preview": "Untitled thread",
  "modelProvider": "openai",
  "createdAt": 1770000000,
  "updatedAt": 1770000100,
  "status": { "type": "idle", "activeFlags": [] },
  "workspaceId": "ws_abc123",
  "rootId": "root_abc123",
  "cwd": "/Users/pz/w/gode",
  "name": "optional title",
  "turns": []
}
```

`workspace` is a named project container owned by the app-server. A workspace
has one or more admitted filesystem roots:

```json
{
  "id": "ws_abc123",
  "name": "gode",
  "roots": [
    { "id": "root_abc123", "path": "/Users/pz/w/gode", "name": "gode" }
  ],
  "defaultRootId": "root_abc123",
  "updatedAt": 1770000100
}
```

`root` is an absolute existing directory registered in a workspace. The server
canonicalizes roots, collapses duplicate root paths within a workspace, and
generates stable ids from canonical paths. Roots are plain `{ id, path, name }`;
capabilities such as VCS provider or package manager are discovered separately.

`cwd` is the execution directory for a thread. `thread/start` derives it from
the selected root by default. If a client supplies an explicit `cwd`, it must be
the selected root or a child path of that root.

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

`item` is a typed visible row or event within a turn. Public item `type` values
are `userMessage`, `agentMessage`, `reasoning`, `toolExecution`, `compaction`,
`error`, and `raw`.

`provider` is an inference backend. Provider/model notation is exposed as a
provider id plus model id, for example `openai` and `gpt-5.5`, or provider
catalog entries that intentionally use Codex provider IDs.

`mode` is Roder's policy mode. App-server clients see it in `thread/state` and
can change it with `thread/set_mode` or `settings/set_default_mode`.

## Method Index

Core:

| Method | Purpose |
| --- | --- |
| `initialize` | Startup handshake with active provider, model, and cwd. |
| `extensions/list` | List extension manifests and capability status. |
| `providers/list` | List providers, auth status, capabilities, and models. |
| `providers/configure` | Persist an API key for an API-key provider. |
| `providers/select` | Select active default provider/model/reasoning. |
| `model/list` | List protocol model descriptors. |
| `settings/get` | Read hosted web search mode, search-index status, shell command shell, default policy mode, and file-backed context status. |
| `settings/set_web_search` | Set hosted web search mode. |
| `settings/set_search_index` | Enable or disable the persistent regex search index. |
| `settings/set_shell` | Set the shell used by the `shell` tool and default `exec_command` calls. |
| `settings/set_default_mode` | Set default policy mode. |
| `settings/set_file_backed_dynamic_context` | Enable or disable file-backed dynamic context. |
| `auth/codex/login` | Start Codex OAuth login. |
| `auth/codex/status` | Read Codex OAuth status. |
| `auth/codex/logout` | Clear Codex OAuth credentials. |
| `auth/supergrok/login` | Start SuperGrok OAuth login. |
| `auth/supergrok/status` | Read SuperGrok OAuth status. |
| `auth/supergrok/logout` | Clear SuperGrok OAuth credentials. |
| `speech/providers/list` | List speech transcription providers and models. |
| `speech/transcribe` | Transcribe audio through a registered speech provider. |
| `speech/synthesis/providers/list` | List speech synthesis providers and TTS models. |
| `speech/synthesize` | Generate speech audio through a registered synthesis provider. |

Threads and turns:

| Method | Purpose |
| --- | --- |
| `thread/start` | Create a thread. |
| `thread/list` | List threads. |
| `thread/read` | Read a thread with optional turns. |
| `thread/archive` | Archive a thread and remove it from active listings. |
| `thread/goal/get` | Read the thread goal state. |
| `thread/goal/set` | Create or update the thread goal state. |
| `thread/goal/clear` | Clear the thread goal state. |
| `turn/start` | Start a turn from rich text input. |
| `turn/steer` | Add user input to an active turn. |
| `turn/interrupt` | Interrupt an active turn. |
| `thread/state` | Read policy mode and pending plan-exit state. |
| `thread/set_mode` | Set the live policy mode. |
| `thread/exit_plan` | Resolve a pending plan-exit request. |
| `thread/resolve_approval` | Resolve a pending tool approval request. |
| `thread/resolve_user_input` | Resolve a pending model-requested user input request. |

Tools, commands, files, agents, and tasks:

| Method | Purpose |
| --- | --- |
| `tools/list` | List runtime tool specs. |
| `discovery/refresh` | Rebuild the lazy discovery catalog. |
| `discovery/groups` | List lazy discovery catalog groups. |
| `discovery/search` | Search lazy discovery items. |
| `discovery/read` | Read and optionally promote one discovery item. |
| `discovery/promote` | Promote one discovery item for a thread. |
| `discovery/promoted/list` | List promoted discovery items. |
| `discovery/promoted/clear` | Clear promoted discovery state. |
| `skills/list` | List skill descriptors and diagnostics visible to the runtime. |
| `skills/read` | Read one skill body by exact selector. |
| `skills/setEnabled` | Persist a skill enable/disable rule. |
| `skills/setExposure` | Persist a skill exposure rule. |
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
| `workflows/plan` | Draft and validate a dynamic workflow run without starting child agents. |
| `workflows/approve` | Approve or deny a planned workflow run. |
| `workflows/list` | List dynamic workflow run summaries. |
| `workflows/get` | Read one workflow run, optionally including script body and agents. |
| `workflows/pause` | Pause a workflow and optionally cancel running child agents. |
| `workflows/resume` | Resume a paused workflow. |
| `workflows/stop` | Stop a workflow run. |
| `workflows/restartAgent` | Restart one child agent in a workflow run. |
| `workflows/save` | Save a workflow run script as a reusable command. |
| `workflows/scripts/list` | List saved and built-in workflow command scripts. |
| `workflows/scripts/read` | Read one workflow command script. |
| `workflows/scripts/delete` | Delete a saved workflow command script. |
| `webwright/prepare` | Create a Webwright workspace with starter artifacts. |
| `webwright/submit` | Submit a Webwright browser task. |
| `webwright/artifacts` | Read a structured Webwright workspace summary. |
| `webwright/latestRun` | Read the latest Webwright run summary. |
| `webwright/verify` | Verify the latest Webwright run artifacts. |
| `webwright/report` | Read Task2UI-style report data. |
| `webwright/rerun` | Rerun a workspace `final_script.py` as a process task. |
| `webwright/setup` | Create a managed Python Playwright runtime and install a selected browser. |
| `webwright/export` | Export a sanitized Webwright workspace package. |
| `webwright/visualJudge` | Optionally judge the latest screenshot with the active image-capable provider. |
| `processes/list` | List Roder-owned local and remote processes. |
| `processes/get` | Read one process descriptor plus output tail. |
| `processes/stop` | Stop one Roder-owned process. |
| `processes/stopAll` | Stop every stoppable Roder-owned process. |
| `processes/subscribe` | Return supported process event kinds. |
| `automations/status` | Read scheduler ownership, store path, and run counters. |
| `automations/list` | List automation definitions. |
| `automations/create` | Register a scheduled Roder run. |
| `automations/update` | Patch an automation definition. |
| `automations/delete` | Disable an automation while preserving run history. |
| `automations/runNow` | Queue an immediate automation run. |
| `automations/runs` | Read automation run history. |
| `automations/cancelRun` | Cancel a queued or running automation run. |
| `eval/reports/list` | List bounded eval report summaries from the workspace. |
| `eval/report/read` | Read one bounded eval report markdown body. |

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

Roadmaps:

| Method | Purpose |
| --- | --- |
| `roadmap/list` | List roadmap Markdown documents in the workspace. |
| `roadmap/read` | Parse one roadmap document. |
| `roadmap/create` | Create a numbered `roadmap/{NN}-{slug}.md` plan. |
| `roadmap/patch` | Replace one exact text span in a roadmap document. |
| `roadmap/task/update` | Update one task checkbox; marking done requires evidence. |
| `roadmap/validate` | Validate one roadmap or all workspace roadmaps. |
| `roadmap/thread/list` | List threads attached to the active roadmap state. |
| `roadmap/thread/spawn` | Create and attach a generated roadmap thread id. |
| `roadmap/thread/attach` | Attach an existing thread id to a roadmap task. |
| `thread/roadmap/open` | Open a roadmap and enter roadmapping mode. |
| `thread/attach` | Alias for attaching an existing thread id to a roadmap task. |

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
| `vcs/status` | Read active version-control provider status and capabilities. |
| `vcs/changes/list` | List live provider changes against the resolved base. |
| `vcs/changes/read` | Read paged changed content for one provider-relative file. |
| `vcs/snapshot/create` | Create a provider history snapshot for selected paths. |
| `vcs/restore` | Restore selected paths where the provider supports it. |
| `vcs/lines/list` | List provider lines of work such as git branches. |
| `vcs/lines/switch` | Switch provider line of work when safe. |
| `vcs/sync` | Run provider sync operations such as fetch, pull, or push. |
| `workspace/list` | List registered workspaces and roots. |
| `workspace/create` | Create or replace a workspace from one or more roots. |
| `workspace/update` | Rename a workspace, replace roots, or change its default root. |
| `workspace/forget` | Remove a workspace registry entry. |
| `hunk/list` | List recorded hunks, optionally by turn/review. |
| `hunk/read` | Read a paged hunk diff. |
| `hunk/rollback` | Confirm and apply a hunk reverse patch. |
| `workspace/changes/list` | List observed VCS-reconciled shell/exec changes. |
| `workflow/scan` | Scan workflow imports. |
| `workflow/preview` | Preview workflow import items. |
| `workflow/enable` | Enable a workflow import. |
| `workflow/ignore` | Ignore a workflow import. |
| `workflow/refresh` | Re-scan and detect stale enabled imports. |
| `workflow/remove` | Remove an enabled workflow import decision. |
| `marketplaces/list` | List plugin marketplace descriptors. |
| `marketplaces/install_default` | Install one or all baked-in marketplace descriptors. |
| `marketplaces/add` | Add a local plugin marketplace descriptor. |
| `marketplaces/remove` | Remove a custom marketplace or disable a baked-in default. |
| `marketplaces/refresh` | Read and normalize a marketplace catalog. |
| `marketplaces/search` | Search de-duplicated marketplace plugins. |
| `marketplaces/plugin` | Read one marketplace plugin variant. |
| `plugins/preview_install` | Preview plugin install metadata and risk hints. |
| `plugins/install` | Record an installed marketplace plugin variant. |
| `plugins/install_all_variants` | Install every variant in a de-duplicated plugin group. |
| `plugins/list_installed` | List installed marketplace plugin variants. |
| `plugins/disable` | Mark an installed marketplace plugin variant disabled. |
| `plugins/uninstall` | Remove an installed marketplace plugin variant record. |
| `media/list` | List media artifacts. |
| `media/read` | Read artifact bytes as base64. |
| `media/thumbnail` | Read an artifact preview. |
| `media/delete` | Delete an artifact. |
| `media/attachToTurn` | Convert an artifact to a turn attachment/image. |
| `artifact/list` | List thread-scoped file-backed context artifacts. |
| `artifact/read` | Read a paged artifact line range. |
| `artifact/grep` | Search an artifact with a literal query. |
| `artifact/tail` | Read the final lines of an artifact. |
| `artifact/delete` | Delete a Roder-owned context artifact. |
| `search_index/status` | Read persistent regex index state for a workspace. |
| `search_index/warmup` | Build the regex index if it is missing. |
| `search_index/rebuild` | Force rebuild the regex index. |
| `search_index/clear` | Remove the regex index store. |
| `index/status` | Read semantic code-index state and generation metadata. |
| `index/rebuild` | Rebuild the semantic code index. |
| `index/search` | Query proof-verified semantic code chunks. |
| `index/readChunk` | Read source for a specific code-index chunk. |
| `index/proofs/list` | List content proofs for indexed chunks. |
| `retrieval/recommendations` | Read retrieval router recommendations for a turn. |
| `retrieval/metrics` | Read retrieval outcome metrics for a turn. |
| `retrieval/promoted` | Read discovery/promotion state for a turn. |
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

### `workflows/plan`

Purpose: Draft a dynamic workflow script, validate it, and return an
approval-gated run without launching child agents.

Request:

```json
{
  "threadId": "thread-123",
  "turnId": "turn-123",
  "prompt": "use a workflow to audit auth flows",
  "workspace": "/Users/example/project",
  "arguments": { "scope": "auth" }
}
```

Response:

```json
{
  "run": {
    "runId": "workflow-123",
    "status": "awaitingApproval",
    "script": {
      "name": "planned-workflow",
      "source": { "kind": "generated" },
      "hostApiVersion": 1
    },
    "phases": [{ "phaseId": "run", "name": "run", "status": "queued" }]
  },
  "approvalRequired": true
}
```

Behavior:

- Emits `workflows/drafted` and `workflows/approvalRequested`.
- `script` may be provided by clients that already have a workflow script;
  otherwise the app-server generates a constrained script from `prompt`.
- The script is validated before approval. The workflow runtime may coordinate
  child agents only through host APIs; scripts cannot directly run shell,
  access files, read secrets, use network, or call MCP.
- `arguments` are stored with the draft and passed to the runner only after
  approval.

Errors:

- Invalid JavaScript, unsupported host API versions, or denied ambient APIs
  return JSON-RPC error code `-32000` with details.

### `workflows/approve`

Purpose: Resolve the approval gate for a planned workflow run.

Request:

```json
{
  "runId": "workflow-123",
  "decision": "runOnce",
  "reason": "approved from desktop"
}
```

Response:

```json
{
  "approval": {
    "runId": "workflow-123",
    "decision": "runOnce",
    "approvedCapabilities": ["childAgents"]
  },
  "run": { "runId": "workflow-123", "status": "running" }
}
```

Behavior:

- `decision` is one of `runOnce`, `alwaysForScriptAndWorkspace`, or `deny`.
- Approving emits `workflows/approved`, `workflows/queued`, then runner
  lifecycle notifications such as `workflows/started`,
  `workflows/phaseStarted`, `workflows/agentStarted`, and terminal
  completion/failure events.
- Denying emits `workflows/denied` and returns a terminal failed run without
  launching child agents.

Errors:

- Unknown run ids or runs that are no longer awaiting approval return
  JSON-RPC error code `-32000`.

### `workflows/list` and `workflows/get`

Purpose: Render live or historical workflow progress.

Request:

```json
{ "threadId": "thread-123", "includeTerminal": true }
```

Response:

```json
{
  "runs": [
    {
      "runId": "workflow-123",
      "status": "completed",
      "title": "planned-workflow",
      "phaseCount": 2,
      "completedPhaseCount": 2,
      "agentCount": 12,
      "completedAgentCount": 11,
      "failedAgentCount": 1,
      "concurrencyPeak": 4,
      "elapsedMs": 12000
    }
  ]
}
```

`workflows/get` uses:

```json
{
  "runId": "workflow-123",
  "includeScriptBody": true,
  "includeAgents": true
}
```

Behavior:

- Summaries include phase counts, agent counts, failure counts, concurrency
  peak, elapsed time, report preview, and token usage when child agents report
  usage.
- `includeScriptBody` controls whether the returned `WorkflowScript` includes
  the script source body. `includeAgents` controls whether child agent rows are
  included.

### Workflow control methods

Purpose: Control an active or retained workflow run.

Requests:

```json
{ "runId": "workflow-123", "cancelRunningAgents": false, "reason": "user pause" }
```

```json
{ "runId": "workflow-123" }
```

```json
{ "runId": "workflow-123", "reason": "user stop" }
```

```json
{ "runId": "workflow-123", "agentId": "agent-1" }
```

Methods:

- `workflows/pause` pauses new child-agent launches and can request running
  child cancellation with `cancelRunningAgents`.
- `workflows/resume` resumes a paused run and reuses retained child results.
- `workflows/stop` emits a terminal stopped run.
- `workflows/restartAgent` invalidates and reruns one child agent when the run
  is still active.

Errors:

- Unknown run ids and invalid status transitions return JSON-RPC error code
  `-32000`.

### Workflow script methods

Purpose: Save, list, read, and delete reusable workflow command scripts.

Requests:

```json
{
  "runId": "workflow-123",
  "name": "audit-workflow",
  "scope": "workspace",
  "overwrite": false
}
```

```json
{ "workspace": "/Users/example/project", "includeUser": true, "includeBuiltin": true }
```

```json
{ "name": "deep-research", "source": "builtIn", "includeBody": true }
```

Behavior:

- `workflows/save` writes `.workflow.js` scripts either to the workspace
  workflow directory or the user workflow directory. Existing scripts are not
  replaced unless `overwrite` is true.
- `workflows/scripts/list` returns built-in scripts, user scripts, and
  workspace scripts according to the include flags.
- `workflows/scripts/delete` can remove a saved script record and, when
  requested, the backing file. Built-in scripts are read-only.

Notes:

- Dynamic workflow methods use the plural `workflows/*` namespace. Singular
  `workflow/*` methods remain reserved for workflow-import scanning and enable
  decisions.

### `initialize`

Purpose: Perform the app-server startup handshake.

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
      "capabilities": {
        "streaming": true,
        "tool_calls": true,
        "tool_search": true,
        "image_input": true
      },
      "models": []
    }
  ]
}
```

Behavior:

- Providers are sorted by `sortOrder`, then name.
- OAuth providers report `authenticated` by checking the relevant token store.
- `capabilities.tool_search` means the provider can map Roder's canonical
  provider-native tool-search hint into its native request body. It does not
  bypass Roder tool permissions, hooks, policy modes, or transcript events.
- OpenRouter is exposed as provider `openrouter`; its built-in fallback model
  id is `x-ai/grok-build-0.1`, so clients should keep provider and model fields
  separate instead of splitting model ids on every slash.
- Model listing failures for an individual provider are treated as an empty
  model list.

### `providers/configure`

Purpose: Persist an API key for a registered API-key provider.

Request:

```json
{
  "provider": "poolside",
  "api_key": "sk-..."
}
```

Response:

```json
{
  "provider": "poolside",
  "authenticated": true
}
```

Behavior:

- Requires the provider to be registered in the runtime inference registry.
- OpenRouter API keys are configured with provider `openrouter`; optional
  attribution headers are read from config or environment, not from this method.
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

### `speech/synthesis/providers/list`

Purpose: Discover registered text-to-speech providers, auth state, synthesis
capabilities, and TTS models.

Request:

```json
{}
```

Response:

```json
{
  "providers": [
    {
      "id": "xiaomi-mimo",
      "name": "Xiaomi MiMo Speech Synthesis",
      "authType": "api_key",
      "authLabel": "MIMO_API_KEY",
      "authenticated": false,
      "capabilities": {
        "batch": true,
        "streaming": false,
        "builtinVoices": true,
        "voiceDesign": true,
        "voiceClone": true,
        "prompt": true
      },
      "models": [
        {
          "id": "mimo-v2.5-tts",
          "name": "MiMo V2.5 TTS"
        }
      ]
    }
  ]
}
```

Behavior:

- Providers are sorted by `sortOrder`, then name.
- Model listing failures for an individual provider are treated as an empty
  model list.
- Xiaomi MiMo TTS providers share provider ids with the corresponding billing
  provider: `xiaomi-mimo` and `xiaomi-mimo-token-plan`.

### `speech/synthesize`

Purpose: Generate audio from text through a registered speech synthesis
provider.

Request:

```json
{
  "provider": "xiaomi-mimo",
  "model": "mimo-v2.5-tts",
  "text": "Hello from Roder.",
  "voice": "Chloe",
  "audioFormat": "wav",
  "prompt": "Warm, clear narration."
}
```

Response:

```json
{
  "provider": "xiaomi-mimo",
  "model": "mimo-v2.5-tts",
  "audio": {
    "bytesBase64": "...",
    "mimeType": "audio/wav",
    "filename": null
  },
  "durationMillis": null,
  "providerResponseId": "chat-response-id",
  "metadata": {}
}
```

Behavior:

- If `provider` is omitted, the first registered speech synthesis provider is
  used.
- If `model` is omitted, the provider's first listed synthesis model is used.
- `voiceSample` accepts the same `bytesBase64`, `mimeType`, and `filename`
  shape as speech transcription audio payloads.

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
  "search_index": { "enabled": true },
  "shell": { "shell": "bash", "options": ["zsh", "bash"] },
  "default_provider": "openai",
  "default_model": "gpt-5.5",
  "default_reasoning": "medium",
  "default_mode": "default",
  "file_backed_dynamic_context": true
}
```

Notes:

- `web_search.mode` is one of `disabled`, `cached`, or `live`.
- `search_index.enabled` controls the persistent regex search-index methods.
- `shell.shell` is the active shell used by the `shell` tool and by
  `exec_command` calls that do not pass a `shell` override. `shell.options`
  lists selectable shells for UI clients.
- `default_provider`, `default_model`, `default_reasoning`, and `default_mode`
  initialize client controls; per-turn overrides are supplied to `turn/start`.
- `default_mode` is a `PolicyMode` value from `roder-api`.
- `file_backed_dynamic_context` controls whether long tool output, command
  output, and compaction source material are written to context artifacts.

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

### `settings/set_search_index`

Purpose: Enable or disable the persistent regex search index used by fast
workspace grep-style lookups.

Request:

```json
{
  "enabled": false
}
```

Response:

```json
{
  "search_index": { "enabled": false }
}
```

Behavior:

- Updates the process-wide search-index setting immediately.
- Persists the setting to user config when app-server config persistence is
  enabled.
- Publishes `search_index/statusChanged` after the setting changes so clients
  can refresh index status displays.

### `settings/set_shell`

Purpose: Change the shell used by command execution tools.

Request:

```json
{
  "shell": "zsh"
}
```

Response:

```json
{
  "shell": { "shell": "zsh", "options": ["zsh", "bash"] }
}
```

Behavior:

- Updates runtime state immediately for future `shell` tool calls and
  `exec_command` calls without an explicit `shell` parameter.
- `exec_command.shell` remains a per-call override.
- Persists `[tools].shell` when app-server config persistence is enabled.

Errors:

- Empty `shell` returns JSON-RPC code `-32602`.

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

### `settings/set_file_backed_dynamic_context`

Purpose: Enable or disable file-backed dynamic context.

Request:

```json
{
  "enabled": false
}
```

Response:

```json
{
  "enabled": false
}
```

Behavior:

- Updates runtime state immediately.
- When disabled, Roder falls back to bounded inline truncation and local
  compaction summaries without writing new context artifacts.
- Existing artifacts remain readable through `artifact/*` methods.
- Persists `[context].file_backed_dynamic_context` when user-config persistence
  is enabled.

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

### Workspace registry methods

Purpose: Register project containers and their admitted filesystem roots.

Examples:

```json
{
  "method": "workspace/create",
  "params": {
    "name": "gode",
    "roots": [
      { "path": "/Users/pz/w/gode", "name": "backend" },
      { "path": "/Users/pz/w/gode-desktop", "name": "desktop" }
    ],
    "defaultRootPath": "/Users/pz/w/gode"
  }
}
```

```json
{
  "method": "workspace/update",
  "params": {
    "workspaceId": "ws_abc123",
    "name": "gode platform",
    "defaultRootId": "root_desktop"
  }
}
```

Behavior:

- `workspace/list` returns persisted workspaces. When the runtime has a
  workspace path, the registry seeds an initial single-root workspace for it.
- `workspace/create` accepts one or more root inputs and returns the resulting
  `Workspace`. Root paths must be absolute existing directories. Duplicate
  canonical paths collapse to one root.
- `workspace/update` can rename the workspace, replace the root set, and set
  `defaultRootId`. Replacing roots regenerates root ids from canonical paths and
  preserves id stability for roots that remain in the workspace.
- `workspace/forget` removes the registry entry. It does not delete files or
  thread history.

Errors:

- Relative paths, missing directories, empty root sets, unknown workspace ids,
  and unknown default/root ids return JSON-RPC error code `-32602`.

### `thread/start`

Purpose: Create a thread.

Request:

```json
{
  "workspaceId": "ws_abc123",
  "rootId": "root_abc123",
  "model": "gpt-5.5",
  "modelProvider": "openai",
  "reasoning": "high",
  "ephemeral": false
}
```

Response:

```json
{
  "thread": {
    "id": "thread-123",
    "preview": "Untitled thread",
    "modelProvider": "openai",
    "model": "gpt-5.5",
    "createdAt": 1770000000,
    "updatedAt": 1770000000,
    "status": {
      "type": "idle",
      "activeTurnId": null,
      "activeFlags": []
    },
    "workspaceId": "ws_abc123",
    "rootId": "root_abc123",
    "cwd": "/Users/pz/w/gode"
  },
  "model": "gpt-5.5",
  "modelProvider": "openai",
  "reasoning": "high",
  "workspaceId": "ws_abc123",
  "rootId": "root_abc123",
  "cwd": "/Users/pz/w/gode"
}
```

Behavior:

- Creates a persisted runtime thread in a registered workspace/root with
  optional provider/model.
- `workspaceId` is required. `rootId` is optional and defaults to the
  workspace's `defaultRootId`.
- `cwd` is optional. When omitted, it defaults to the selected root path. When
  supplied, it must be the selected root or a child path of that root.
- Stores the selected provider/model/reasoning for later `turn/start` overrides.
- If `reasoning` is omitted, returns and stores the effective reasoning effort for the selected model.
- Emits `thread/started`.
- `ephemeral` is accepted by the DTO but is not currently used by the handler.

### `thread/list`

Purpose: Bootstrap or refresh a thread list.

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
      "preview": "Fix tests",
      "modelProvider": "openai",
      "createdAt": 1770000000,
      "updatedAt": 1770000100,
      "status": { "type": "idle", "activeTurnId": null, "activeFlags": [] },
      "workspaceId": "ws_abc123",
      "rootId": "root_abc123",
      "cwd": "/Users/pz/w/gode",
      "name": "Fix tests"
    }
  ],
  "nextCursor": null,
  "backwardsCursor": null
}
```

Behavior:

- Lists persisted runtime threads sorted by newest `updatedAt` first.
- Applies `limit` when supplied.
- Merges in protocol threads that are in memory but not yet persisted.
- Threads include `workspaceId`, `rootId`, and `cwd` when persisted metadata has
  workspace registry data.
- Cursor fields are currently always null.

### `thread/read`

Purpose: Read one thread and optionally include turns/items.

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
    "preview": "Fix tests",
    "modelProvider": "openai",
    "createdAt": 1770000000,
    "updatedAt": 1770000100,
    "status": { "type": "idle", "activeTurnId": null, "activeFlags": [] },
    "workspaceId": "ws_abc123",
    "rootId": "root_abc123",
    "cwd": "/Users/pz/w/gode",
    "usage": {
      "prompt_tokens": 100,
      "completion_tokens": 10,
      "total_tokens": 110,
      "cached_prompt_tokens": 92,
      "cache_hit_rate": 0.92
    },
    "turns": [
      {
        "id": "turn-123",
        "items": [],
        "itemsView": "default",
        "status": "completed",
        "usage": {
          "prompt_tokens": 100,
          "completion_tokens": 10,
          "total_tokens": 110,
          "cached_prompt_tokens": 92,
          "cache_hit_rate": 0.92
        }
      }
    ]
  }
}
```

Behavior:

- Reads a persisted thread snapshot first.
- Falls back to persisted thread metadata and then in-memory protocol threads.
- Includes aggregate thread `usage` and per-turn `usage` when provider usage
  was reported; `cache_hit_rate` is `cached_prompt_tokens / prompt_tokens`.
- Returns `{"thread": null}` when the thread is unknown.

### `thread/archive`

Purpose: Archive a thread and remove it from active app-server thread
lists.

Request:

```json
{
  "threadId": "thread-123"
}
```

Response:

```json
{
  "threadId": "thread-123",
  "archived": true
}
```

Behavior:

- Calls the runtime thread archive path for the supplied `threadId`.
- Removes in-memory protocol thread, selected model, and active-turn state for
  the thread.
- After archive, `thread/list` no longer returns the thread and `thread/read`
  returns `{ "thread": null }`.

Errors:

- Thread-store or archive failures return code `-32000` with `data.details`.

### `thread/goal/get`

Purpose: Read the durable goal state for a thread.

Request:

```json
{
  "threadId": "thread-123"
}
```

Response:

```json
{
  "goal": {
    "threadId": "thread-123",
    "objective": "Ship the goal parity slice",
    "status": "active",
    "tokenBudget": 20000,
    "tokensUsed": 1200,
    "timeUsedSeconds": 180,
    "createdAt": "2026-05-22T09:00:00Z",
    "updatedAt": "2026-05-22T09:03:00Z"
  }
}
```

Behavior:

- Returns `{ "goal": null }` when no goal is set.
- Goal status is one of `active`, `paused`, `blocked`, `usageLimited`,
  `budgetLimited`, or `complete`.

### `thread/goal/set`

Purpose: Create or update the durable goal state for a thread.

Request:

```json
{
  "threadId": "thread-123",
  "objective": "Ship the goal parity slice",
  "status": "active",
  "tokenBudget": 20000
}
```

Response:

```json
{
  "goal": {
    "threadId": "thread-123",
    "objective": "Ship the goal parity slice",
    "status": "active",
    "tokenBudget": 20000,
    "tokensUsed": 0,
    "timeUsedSeconds": 0,
    "createdAt": "2026-05-22T09:00:00Z",
    "updatedAt": "2026-05-22T09:00:00Z"
  }
}
```

Behavior:

- Creates a goal when `objective` is supplied and no goal exists.
- Updates only supplied fields when a goal already exists.
- `objective` must be non-empty and at most 4000 characters.
- `tokenBudget`, when supplied, must be positive. Send `null` to clear the
  budget.
- Emits `thread/goal/updated` after a goal is created or updated.

### `thread/goal/clear`

Purpose: Clear the durable goal state for a thread.

Request:

```json
{
  "threadId": "thread-123"
}
```

Response:

```json
{
  "cleared": true
}
```

Behavior:

- Returns `false` when no goal existed.
- Emits `thread/goal/cleared` when a goal was removed.

### `turn/start`

Purpose: Start a turn on a thread.

Request:

```json
{
  "threadId": "thread-123",
  "input": [
    { "type": "text", "text": "inspect this repo" }
  ],
  "modelProvider": "openai",
  "model": "gpt-5.5",
  "reasoning": "high",
  "policyMode": "default"
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
- If the thread already has an active runtime turn, queues the input as
  same-turn steering and returns that active `turnId`.
- Otherwise uses explicit model/provider/reasoning overrides first, then the
  thread selection, then runtime defaults. If `policyMode` is supplied, applies
  it as the live policy mode before starting the turn.
- Starts a runtime turn with the thread's persisted workspace and records the
  active turn id for optional `turn/interrupt`.

Notifications:

- `turn/started`
- `thread/status/changed` with status `running`
- zero or more typed item-event notifications such as `item/started`,
  `item/agentMessage/delta`, `item/reasoning/textDelta`, and `item/completed`
- optional wait-state notifications: `thread/approvalRequested`,
  `thread/userInputRequested`, or `thread/planExitRequested`, paired with
  their corresponding resolved notifications when the client answers
- terminal `turn/completed`
- `thread/status/changed` with status `idle`

### `turn/steer`

Purpose: Send additional user input to an active turn.

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

Purpose: Interrupt a turn.

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

### `thread/state`

Purpose: Read current policy mode and any pending plan-exit request.

Request:

```json
{}
```

Response:

```json
{
  "mode": "plan",
  "pendingPlanExit": {
    "threadId": "thread-123",
    "turnId": "turn-123",
    "requestId": "request-123",
    "targetMode": "default",
    "planSummary": "Implement the test first.",
    "requestedAt": "2026-05-18T12:00:00Z",
    "expiresAt": null
  }
}
```

### `thread/set_mode`

Purpose: Set the live policy mode.

Request:

```json
{
  "mode": "accept_edits",
  "reason": "client toggle"
}
```

Response:

```json
{
  "mode": "accept_edits"
}
```

### `thread/exit_plan`

Purpose: Approve or reject a pending plan-mode exit.

Request:

```json
{
  "requestId": "request-123",
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

### `thread/resolve_approval`

Purpose: Resolve a pending tool approval.

Request:

```json
{
  "approvalId": "approval-123",
  "approved": true
}
```

Response:

```json
{
  "resolved": true
}
```

### `thread/resolve_user_input`

Purpose: Resolve a pending `request_user_input` tool request.

Request:

```json
{
  "requestId": "input-123",
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
  "stderr": "",
  "stdoutArtifact": {
    "id": "artifact-123",
    "kind": "command_stdout",
    "threadId": "app-server",
    "turnId": "process-123",
    "byteCount": 2097152,
    "lineCount": 1200,
    "sourceToolId": "process-123",
    "label": "stdout",
    "createdAt": "2026-05-20T18:00:00Z"
  }
}
```

Response when `streamStdoutStderr` is true:

```json
{
  "exitCode": 0,
  "stdout": "",
  "stderr": "",
  "stdoutArtifact": {
    "id": "artifact-123",
    "kind": "command_stdout",
    "threadId": "app-server",
    "turnId": "process-123",
    "byteCount": 2097152,
    "lineCount": 1200,
    "sourceToolId": "process-123",
    "label": "stdout",
    "createdAt": "2026-05-20T18:00:00Z"
  }
}
```

Behavior:

- Requires `command` to be non-empty.
- `cwd` must be absolute when supplied.
- Default timeout is 30000 ms unless `disableTimeout` is true.
- Default output cap is 1048576 bytes unless `disableOutputCap` is true.
- When streaming is enabled, `processId` is required and stdout/stderr are sent
  as `command/exec/outputDelta` notifications.
- When stdout or stderr exceeds the output cap, the capped inline stream is
  preserved and the full stream is written as a context artifact. Responses
  include `stdoutArtifact` or `stderrArtifact` descriptors when those artifacts
  are created. Command artifacts are scoped to thread `app-server` and turn
  `processId`.
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

- Only `get_goal`, `create_goal`, and `update_goal` can be called directly.
- Other tool names return code `-32602`.

### Discovery methods

Purpose: Inspect and promote lazily discovered tools, skills, workflows,
artifacts, and other capability descriptions without loading the full catalog
into every model prompt.

Examples:

```json
{
  "method": "discovery/search",
  "params": {
    "query": "grep",
    "groupId": "tools",
    "limit": 10
  }
}
```

```json
{
  "method": "discovery/read",
  "params": {
    "itemId": "tool:builtin-coding-tools/grep",
    "promote": true,
    "threadId": "thread-123",
    "turnId": "turn-123"
  }
}
```

Behavior:

- `discovery/refresh` rebuilds the catalog from the runtime's registered
  discovery providers.
- `discovery/groups` returns catalog group metadata for client filters.
- `discovery/search` returns bounded matching items and auth/redaction status
  when the provider exposes it.
- `discovery/read` returns the full item payload and can promote the item into
  a thread/turn when `promote` is true.
- `discovery/promote`, `discovery/promoted/list`, and
  `discovery/promoted/clear` manage promoted capability state for model
  context.

Notifications:

- Discovery runtime events are forwarded as `discovery/catalogBuilt`,
  `discovery/itemUpdated`, `discovery/authRequired`, `discovery/itemRead`,
  `discovery/itemPromoted`, `discovery/promotionReused`,
  `discovery/warmCacheHit`, and `discovery/promotionExpired` when the
  underlying runtime emits them.

### Skills methods

Purpose: List, read, and configure the skill catalog visible to the runtime.

List:

```json
{
  "method": "skills/list",
  "params": {}
}
```

```json
{
  "skills": [
    {
      "id": "builtin:vcs-snapshot",
      "name": "vcs-snapshot",
      "canonicalPath": "roder-builtin://vcs-snapshot/SKILL.md",
      "source": "builtIn",
      "exposure": "direct_only",
      "activation": "enabled",
      "description": "Create scoped VCS snapshots from the current workspace state.",
      "experimental": false
    }
  ],
  "diagnostics": []
}
```

Read:

```json
{
  "method": "skills/read",
  "params": {
    "selector": { "path": "roder-builtin://vcs-snapshot/SKILL.md" }
  }
}
```

Mutate:

```json
{
  "method": "skills/setEnabled",
  "params": {
    "selector": { "name": "vcs-snapshot" },
    "enabled": false
  }
}
```

```json
{
  "method": "skills/setExposure",
  "params": {
    "selector": { "path": "roder-builtin://vcs-snapshot/SKILL.md" },
    "exposure": "global"
  }
}
```

Behavior:

- `skills/list` returns `SkillDescriptor` values plus catalog diagnostics.
  Descriptors include `canonicalPath`, `source`, `exposure`, `activation`,
  `description`, optional `shortDescription`, `experimental`, diagnostics, and
  optional `agentMetadata`.
- `skills/read` returns `{ "skill": null }` when the selector does not match
  exactly one skill.
- `selector` is either `{ "name": "..." }` or `{ "path": "..." }`.
- `skills/setEnabled` and `skills/setExposure` persist canonical skill config
  rules when app-server config persistence is enabled and return the refreshed
  `skills` plus `diagnostics`.

Errors:

- Mutating an unknown skill returns code `-32602` with message
  `Invalid params: skill not found`.
- Mutating by ambiguous name returns code `-32602` and a message that lists the
  canonical paths to select by.

Notifications:

- Runtime skill events are forwarded as `skills/catalogLoaded`,
  `skills/configApplied`, `skills/activationResolved`,
  `skills/indexRendered`, `skills/invoked`, `skills/autoActivated`, and
  `skills/skipped` when emitted.

### `commands/list`

Purpose: List available slash commands.

Built-in commands include workflow commands such as `snapshot`, `roadmap`,
`webwright:run`, and `webwright:craft`. The Webwright commands bind the
built-in `webwright` skill during expansion, so clients should preserve returned
context blocks rather than trying to reimplement the browser-agent prompt.

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

### `webwright/setup`, `webwright/prepare`, `webwright/submit`, `webwright/artifacts`, `webwright/latestRun`, `webwright/verify`, `webwright/report`, `webwright/rerun`, `webwright/export`, `webwright/visualJudge`

Purpose: Drive Roder's first-party Webwright browser task workflow through the
app-server without requiring TUI filesystem scraping.

Key request shapes:

```json
{
  "method": "webwright/setup",
  "params": {
    "browser": "firefox",
    "dryRun": false
  }
}
```

```json
{
  "method": "webwright/prepare",
  "params": {
    "task": "Open the fixture page and extract the heading",
    "mode": "run",
    "taskId": "fixture-heading",
    "workspace": "/Users/pz/w/gode"
  }
}
```

```json
{
  "method": "webwright/rerun",
  "params": {
    "workspace": ".roder/webwright/fixture-heading",
    "workspaceRoot": "/Users/pz/w/gode"
  }
}
```

```json
{
  "method": "webwright/export",
  "params": {
    "workspace": ".roder/webwright/fixture-heading",
    "workspaceRoot": "/Users/pz/w/gode",
    "outputDir": ".roder/webwright-exports/fixture-heading"
  }
}
```

```json
{
  "method": "webwright/visualJudge",
  "params": {
    "workspace": ".roder/webwright/fixture-heading",
    "workspaceRoot": "/Users/pz/w/gode",
    "enabled": true
  }
}
```

Setup response shape:

```json
{
  "roderHome": "/Users/pz/.roder",
  "runtimeDir": "/Users/pz/.roder/python/webwright",
  "python": "/Users/pz/.roder/python/webwright/venv/bin/python",
  "browser": "firefox",
  "dryRun": false,
  "installed": true,
  "steps": [
    {
      "label": "install Playwright browser",
      "command": [
        "/Users/pz/.roder/python/webwright/venv/bin/python",
        "-m",
        "playwright",
        "install",
        "firefox"
      ],
      "status": "completed",
      "stdoutTail": "",
      "stderrTail": ""
    }
  ],
  "message": "Webwright runtime installed"
}
```

Behavior:

- `webwright/setup` creates `~/.roder/python/webwright/venv` by default,
  installs the Playwright Python package, installs the selected browser
  (`firefox`, `chromium`, or `webkit`), and writes
  `~/.roder/python/webwright/setup.json`. If `RODER_CONFIG_DIR` or
  `RODER_DATA_DIR` is set, Roder uses that directory instead of `~/.roder`.
  Pass `dryRun: true` to return the exact setup command plan without executing
  processes. Pass `python` to choose the base Python used for `-m venv`.
- `webwright/prepare` writes `webwright.json`, `plan.md`, and
  `final_script.py` under a scoped `.roder/webwright/<task-id>/` workspace.
- `webwright/submit` submits executor `webwright.browser_task` and returns a
  normal task handle. Its dependency preflight uses `RODER_WEBWRIGHT_PYTHON`
  first, then the managed setup runtime, then system Python.
- `webwright/rerun` copies the root `final_script.py` into the next
  `final_runs/run_<n>/` directory and submits executor `process` with that run
  directory as cwd. If `python` is omitted, it uses the same managed runtime
  lookup as `webwright/submit`.
- `webwright/export` copies only shareable Webwright artifacts into a scoped
  export directory, redacts text-file secret lines, writes
  `webwright-export.json`, and excludes browser state, cookies, raw headers, and
  unrelated workspace files by default.
- `webwright/visualJudge` is disabled unless `enabled: true` is passed or
  `RODER_WEBWRIGHT_VISUAL_JUDGE=1` is set. It only calls the active inference
  provider when `providers/list` reports `capabilities.imageInput: true`;
  otherwise it writes a skipped record under `visual_judge/`.
- Visual judge prompts and provider responses are redacted and stored under the
  task workspace as `visual_judge/run_<n>.json`; they are not written to global
  logs.
- Artifact, latest-run, report, and verification methods return structured JSON
  derived from the workspace parser; clients should render these fields instead
  of parsing terminal output. `webwright/report` also includes `renderedText`,
  a redacted plain-text rendering of Task2UI sections for clients without a
  custom report renderer.
- All workspace paths are scoped to `workspaceRoot`, the runtime workspace, or
  the process cwd. Parent-component escapes are rejected.
- Setup rejects unsupported browsers and reports failed `venv`, `pip`, or
  Playwright install commands as JSON-RPC invalid-params errors with captured
  stderr tail in the error message.

### `processes/list`, `processes/get`, `processes/stop`, `processes/stopAll`, `processes/subscribe`

Purpose: inspect and control processes spawned by Roder. This surface only reports Roder-owned processes from `command/exec`, process-backed background tasks, and remote runner commands. It does not enumerate arbitrary host OS processes, and local OS PIDs are metadata only; clients must use `processId`.

List active processes:

```json
{
  "method": "processes/list",
  "params": { "includeCompleted": false }
}
```

Read one process and its retained output tail:

```json
{
  "method": "processes/get",
  "params": { "processId": "task-abc123", "outputBytes": 4096 }
}
```

Stop one process:

```json
{
  "method": "processes/stop",
  "params": { "processId": "task-abc123", "reason": "user requested stop" }
}
```

Stop all active stoppable processes:

```json
{
  "method": "processes/stopAll",
  "params": { "reason": "workspace cleanup" }
}
```

`processes/subscribe` returns these event kinds:

```json
{
  "subscribed": true,
  "eventKinds": [
    "process.started",
    "process.output",
    "process.exited",
    "process.stopping",
    "process.stopped",
    "process.failed"
  ]
}
```

Retention:

- `processes/list` returns active processes by default.
- Pass `includeCompleted: true` to include retained terminal descriptors.
- Completed descriptors and output tails are bounded by the process registry retention limits.

Remote runner semantics:

- Remote process descriptors include `runnerDestinationId` and `runnerSessionId`.
- Remote stops call the runner provider cancellation API through `cancel_command`.
- Remote stops never attempt to kill local host PIDs.

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
thread events.

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

- Missing threads return empty trace lists or empty event pages.
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

### Workspace observed change methods

Purpose: List file-level workspace changes observed after shell/exec tools.

Examples:

```json
{
  "method": "workspace/changes/list",
  "params": {
    "threadId": "thread-123",
    "turnId": "turn-123"
  }
}
```

Behavior:

- `workspace/changes/list` can filter by `turnId`.
- Observed changes are VCS-reconciled file summaries, not exact structured
  hunks. The review panel can read current changed content through `vcs/changes/read`.
- New observed changes emit `workspace/changeObserved`.

### VCS change review methods

Purpose: Inspect the active version-control provider without mutating files.

Examples:

```json
{
  "method": "vcs/changes/list",
  "params": {
    "workspaceId": "ws_abc123",
    "rootId": "root_abc123",
    "limit": 500
  }
}
```

```json
{
  "method": "vcs/changes/read",
  "params": {
    "workspaceId": "ws_abc123",
    "rootId": "root_abc123",
    "path": "src/app.rs",
    "area": "unstaged",
    "ignoreWhitespace": true,
    "offset": 0,
    "limit": 400
  }
}
```

Behavior:

- `vcs/status` returns provider identity, workspace root, active line of work,
  base information when known, and capability metadata.
- VCS methods operate on a registered workspace root. `workspaceId` is required;
  `rootId` is optional and defaults to the workspace default root. Raw absolute
  `workspace` paths are not canonical VCS inputs.
- Provider resolution uses the selected root path, so multi-root projects review
  one root at a time.
- `vcs/changes/list` returns provider status, changed files, totals, and whether
  the file list was truncated.
- The bundled git provider compares the merge-base of the resolved base with the
  current working tree, including committed, staged, unstaged, and untracked
  changes.
- `vcs/changes/read` validates provider-relative paths and returns paged changed
  content for one changed file. When `area` is omitted, it returns the full
  branch delta. When `area` is provided, providers may return just that file's
  `committed`, `staged`, `unstaged`, or `untracked` content.
- `ignoreWhitespace` defaults to `false`; when set, providers may suppress
  whitespace-only changes in the returned diff.
- Mutating calls such as `vcs/snapshot/create`, `vcs/restore`,
  `vcs/select`, `vcs/lines/switch`, and `vcs/sync` are checked by the
  app-server policy gate before execution. If approval is required, the request
  waits for the normal `thread/approvalRequested` / `thread/resolve_approval`
  flow and continues only when approved.
- Mutating calls should also be gated by capability data before invocation.
  Unsupported operations return provider-aware JSON-RPC errors. Provider-native
  extras are represented in capability metadata rather than a separate
  discovery endpoint.

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
- `includeUser: true` also scans `~/.roder`, `~/.agents`, and installed
  marketplace plugin cache markers from `RODER_MARKETPLACES_PATH` or
  `~/.roder/marketplaces.json`.
- Installed marketplace plugins are returned as source-attributed
  `sourceType: "plugin"` items with `variantKey`, `marketplaceId`, `pluginId`,
  `identityKey`, `installPath`, and redacted manifest preview data. MCP,
  hooks, apps, LSP, npm, script, and binary hints set `approvalRequired: true`.
- `enable`, `ignore`, and `remove` persist decisions to
  `~/.roder/workflow-imports.json` unless overridden by
  `RODER_WORKFLOW_IMPORTS_PATH`.
- Enabling an item that requires approval without `approveSideEffects` returns
  code `-32040` with `itemId`, `source`, and `risk` in `data`.

### Automation methods

Purpose: Manage app-server scheduled Roder runs, inspect scheduler ownership,
and read run history. See `docs/app-server/automations.md` for the full
method, config, scheduler, lease-recovery, and client-integration reference.

Example status request:

```json
{
  "method": "automations/status",
  "params": {}
}
```

Example status response:

```json
{
  "schedulerEnabled": false,
  "readApiEnabled": true,
  "serverId": "desktop-main",
  "serverRole": "desktop",
  "storePath": "/Users/example/.roder/automations.sqlite3",
  "activeRuns": 0,
  "dueCount": 0,
  "leasedCount": 0
}
```

Behavior:

- Scheduling is disabled by default. App clients may enable scheduler
  ownership for their app-server process; ordinary TUI-local app servers should
  remain scheduler-disabled unless explicitly requested.
- Disabled scheduler instances can still serve read/manage APIs when
  `readApiEnabled` is true.
- `automations/runNow` uses the same lease, task, thread, turn, event, and run
  audit path as scheduled occurrences.
- Missed runs are represented either as due runs according to catch-up policy or
  as `skipped` run records with `skipReason`.
- Failed runs record `error`; interactive approval or user-input waits also
  emit `automations/needsInput`.

### Plugin marketplace methods

Purpose: Manage Claude, Cursor, Codex, and local plugin marketplace metadata,
search de-duplicated plugin entries, preview plugin installs, and record local
installed plugin variants.

Marketplace state is persisted under `~/.roder/marketplaces.json` unless
`RODER_MARKETPLACES_PATH` is set. Plugin cache markers are written under
`~/.roder/plugins/cache` unless `RODER_MARKETPLACE_CACHE_DIR` is set. Remote
marketplace source checkouts and downloaded JSON catalogs are cached under
`~/.roder/marketplaces/cache` unless `RODER_MARKETPLACE_SOURCE_CACHE_DIR` is
set. Tests and offline fixture runs can map GitHub shorthand sources through
`RODER_MARKETPLACE_GITHUB_FIXTURE_DIR`.

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

Purpose: Add a custom marketplace descriptor.

Request:

```json
{
  "id": "local-cursor",
  "kind": "cursor",
  "displayName": "Local Cursor",
  "source": {
    "kind": "localPath",
    "path": "/Users/pz/plugins/cursor"
  }
}
```

GitHub shorthand source:

```json
{
  "id": "team-plugins",
  "kind": "claude",
  "displayName": "Team Plugins",
  "source": {
    "kind": "github",
    "repo": "example/plugins",
    "refName": "main",
    "catalogPath": ".claude-plugin/marketplace.json"
  }
}
```

Git URL and direct HTTP JSON sources use:

```json
{
  "id": "remote-json",
  "kind": "cursor",
  "displayName": "Remote JSON",
  "source": {
    "kind": "httpJson",
    "url": "https://example.test/marketplace.json"
  }
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
- `kind` is optional. When omitted, the server infers it from local catalog
  markers or remote source hints. Local paths are inspected immediately.
- Explicit `kind` values are `claude`, `cursor`, `codex`, `roder`, or
  `custom`.
- `source.kind` is `localPath`, `github`, `git`, or `httpJson`.
- GitHub `repo` must be `owner/repo`; optional `catalogPath` and `pluginRoot`
  must be relative and must not contain `..`.
- Git and HTTP JSON sources require supported URL schemes. Git accepts
  `https://`, `ssh://`, `git@`, and `file://`; HTTP JSON accepts `https://`,
  `http://`, and `file://`.
- Local `source.path` values must exist when the request is handled.
- GitHub shorthand sources resolve as `https://github.com/{repo}.git` during
  refresh unless `RODER_MARKETPLACE_GITHUB_FIXTURE_DIR` contains a matching
  fixture path.
- Git URL sources are cloned into the marketplace source cache. `refName`
  checks out a branch, tag, or commit after clone/fetch.
- HTTP JSON sources are fetched into the source cache as `marketplace.json`.
  `file://` URLs are supported for tests and offline fixtures.
- Existing marketplace records with the same `id` are rejected to avoid
  ambiguous custom marketplace ids.

Errors:

- Invalid ids, duplicate ids, unsafe source fields, unsupported source schemes,
  or missing local paths return code `-32602`.
- Store read/write failures return code `-32000`.
- Remote clone/fetch/download failures are reported by `marketplaces/refresh`
  as code `-32000`; `marketplaces/add` records the descriptor even when a
  remote source is temporarily unavailable.

#### `marketplaces/remove`

Purpose: Remove one configured marketplace descriptor.

Request:

```json
{
  "marketplaceId": "local-cursor"
}
```

Response:

```json
{
  "removed": true
}
```

Behavior:

- Custom marketplaces are removed from marketplace state.
- Baked-in default marketplaces are retained for discovery, but set to
  `enabled: false` and `state: "removedByUser"`.
- Returns `removed: false` when no matching marketplace id exists.
- Removing a marketplace does not remove installed plugin records that were
  installed from that marketplace.

Errors:

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
- GitHub and git URL marketplaces are cloned or fetched into the source cache
  before reading their catalog.
- Direct HTTP JSON marketplaces download the JSON into the source cache before
  normalization.
- The marketplace record is updated to `state: "refreshed"` with
  `lastRefreshedAt` and `contentHash`.

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
          "homepage": "https://example.com/repo-tools",
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
          "risk": "passive"
        }
      ],
      "relatedCandidates": [],
      "recommendedVariantKey": "local-cursor:repo-tools",
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
- Weak name-only matches remain separate rows and appear in `relatedCandidates`
  instead of being merged.
- `recommendedVariantKey` is the default provider ordering choice for clients
  that want a one-click install; clients can still install a selected variant
  or call `plugins/install_all_variants` for every provider copy.
- Each variant includes source, optional homepage, component hints, capability
  hints, and risk so clients can show an install preview before activation.
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

#### `plugins/install_all_variants`

Purpose: Install every marketplace variant in the same de-duplicated plugin
group as a selected seed plugin.

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
    },
    {
      "marketplaceId": "local-claude",
      "pluginId": "repo-tools-claude",
      "identityKey": {
        "canonicalSlug": "repo-tools",
        "normalizedName": "repo-tools"
      },
      "variantKey": "local-claude:repo-tools-claude",
      "installPath": "/Users/pz/.roder/plugins/cache/local-claude/repo-tools-claude/6ac3f4c6936e",
      "contentHash": "6ac3f4c6936e...",
      "state": "installed",
      "installedAt": "2026-05-18T18:00:00Z"
    }
  ]
}
```

Behavior:

- The selected `marketplaceId` and `pluginId` identify a seed variant.
- The server rebuilds searchable marketplace entries, de-duplicates them by
  identity key, then installs every variant in the seed's de-duped group.
- Existing installed records with the same `variantKey` are replaced
  idempotently.
- Like `plugins/install`, this records cache state only; it does not enable
  workflow imports or execute plugin-provided code.

Errors:

- Missing seed plugins return code `-32004` with message `plugin not found`.
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

#### `plugins/disable`

Purpose: Mark one installed plugin variant disabled while preserving its record.

Request:

```json
{
  "variantKey": "local-cursor:repo-tools"
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
    "state": "disabled",
    "installedAt": "2026-05-18T18:00:00Z"
  }
}
```

Behavior:

- Returns the updated plugin record when the `variantKey` exists.
- Returns `{ "plugin": null }` when no matching installed variant exists.
- Does not delete cache markers or remove marketplace entries.

Errors:

- Store read/write failures return code `-32000`.

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

### Context artifact methods

Purpose: Inspect file-backed dynamic context without giving clients direct
filesystem paths.

List request:

```json
{
  "threadId": "app-server",
  "kind": "command_stdout",
  "limit": 50
}
```

List response:

```json
{
  "artifacts": [
    {
      "id": "artifact-123",
      "kind": "command_stdout",
      "threadId": "app-server",
      "turnId": "process-123",
      "byteCount": 2097152,
      "lineCount": 1200,
      "sourceToolId": "process-123",
      "label": "stdout",
      "createdAt": "2026-05-20T18:00:00Z"
    }
  ]
}
```

Read request:

```json
{
  "threadId": "app-server",
  "artifactId": "artifact-123",
  "startLine": 200,
  "limit": 50
}
```

Read response:

```json
{
  "page": {
    "artifact": {
      "id": "artifact-123",
      "kind": "command_stdout",
      "threadId": "app-server",
      "turnId": "process-123",
      "byteCount": 2097152,
      "lineCount": 1200,
      "sourceToolId": "process-123",
      "label": "stdout",
      "createdAt": "2026-05-20T18:00:00Z"
    },
    "text": "  200: build output",
    "startLine": 200,
    "limit": 50,
    "shown": 50,
    "totalLines": 1200,
    "nextStartLine": 250,
    "truncated": true
  }
}
```

Grep request:

```json
{
  "threadId": "app-server",
  "artifactId": "artifact-123",
  "query": "RECOVERY_TOKEN",
  "offset": 0,
  "limit": 20
}
```

Tail request:

```json
{
  "threadId": "app-server",
  "artifactId": "artifact-123",
  "lines": 100
}
```

Delete request:

```json
{
  "threadId": "app-server",
  "artifactId": "artifact-123"
}
```

Delete response:

```json
{
  "deleted": true
}
```

Behavior:

- `kind` accepts `tool_output`, `command_stdout`, `command_stderr`,
  `terminal_transcript`, `chat_history`, `compaction_source`, and
  `context_provider_dump`.
- `artifact/read`, `artifact/grep`, and `artifact/tail` cap pages at 200 lines.
- Artifact descriptors intentionally omit `storePath`.
- Every method is scoped by `threadId`; a mismatched artifact id returns code
  `-32000` with a message that the artifact does not belong to the thread.
- `artifact/delete` refuses non-Roder-owned artifacts and emits the runtime
  `artifact/deleted` event when deletion succeeds.
- In the normal JSONL thread store, artifact files are stored under
  `<threadStoreDir>/<threadId>/artifacts/<turnId>/`, beside the thread's
  `metadata.json`, `events.jsonl`, and `transcript_items.jsonl`.

### Search index methods

Purpose: Manage the persistent regex search index used by fast exact-text
workspace search.

Status:

```json
{
  "method": "search_index/status",
  "params": {
    "workspace": "/Users/pz/w/gode"
  }
}
```

```json
{
  "status": {
    "state": "ready",
    "enabled": true,
    "workspace": "/Users/pz/w/gode",
    "storeDir": "/Users/pz/.roder/indexes/workspace-id",
    "indexVersion": "1",
    "documentCount": 2048,
    "indexBytes": 7340032,
    "buildTimeMs": 215,
    "stale": false
  }
}
```

Mutations:

```json
{
  "method": "search_index/warmup",
  "params": { "workspace": "/Users/pz/w/gode" }
}
```

```json
{
  "method": "search_index/rebuild",
  "params": { "workspace": "/Users/pz/w/gode" }
}
```

```json
{
  "method": "search_index/clear",
  "params": { "workspace": "/Users/pz/w/gode" }
}
```

Behavior:

- `workspace` is optional; when omitted the server's runtime workspace is used.
- Status `state` is one of `disabled`, `missing`, `building`, `ready`,
  `stale`, `failed`, or `cleared`.
- `search_index/warmup` returns current status when an index already exists;
  otherwise it builds the index.
- `search_index/rebuild` always rebuilds the index for the workspace.
- `search_index/clear` removes the persistent store and returns `cleared` even
  if the store was already absent.
- Search-index storage is rooted under the configured search-index home, or
  the default `~/.roder/indexes` path used by `roder-search`.

Notifications:

- `search_index/statusChanged` is emitted for `building`, `ready`, `cleared`,
  `disabled`, and failure transitions with payload
  `{ "status": SearchIndexStatus }`.

### Code index methods

Purpose: Build and query the source-free semantic code index, then require an
explicit policy-gated source read when a client wants chunk text.

Status and rebuild:

```json
{
  "method": "index/status",
  "params": {
    "workspace": "/Users/pz/w/gode"
  }
}
```

```json
{
  "method": "index/rebuild",
  "params": {
    "workspace": "/Users/pz/w/gode"
  }
}
```

```json
{
  "status": {
    "status": "ready",
    "workspace": "/Users/pz/w/gode",
    "storePath": "/Users/pz/.roder/code-index/workspace-key/code-index.sqlite3",
    "generationId": "generation-123",
    "rootHash": "merkle-root",
    "stale": false,
    "stats": {
      "fileCount": 128,
      "chunkCount": 640,
      "embeddedChunkCount": 640,
      "cachedEmbeddingCount": 512,
      "indexBytes": 10485760
    }
  }
}
```

Search:

```json
{
  "method": "index/search",
  "params": {
    "workspace": "/Users/pz/w/gode",
    "query": "oauth refresh token",
    "limit": 5
  }
}
```

```json
{
  "status": {
    "status": "ready",
    "workspace": "/Users/pz/w/gode",
    "storePath": "/Users/pz/.roder/code-index/workspace-key/code-index.sqlite3",
    "generationId": "generation-123",
    "rootHash": "merkle-root",
    "stale": false,
    "stats": {
      "fileCount": 128,
      "chunkCount": 640,
      "embeddedChunkCount": 640,
      "cachedEmbeddingCount": 512,
      "indexBytes": 10485760
    }
  },
  "response": {
    "generation": {
      "id": "generation-123",
      "status": "ready",
      "workspaceRoot": "/Users/pz/w/gode",
      "rootHash": "merkle-root",
      "configHash": "config-hash",
      "stats": {
        "fileCount": 128,
        "chunkCount": 640,
        "embeddedChunkCount": 640,
        "cachedEmbeddingCount": 512,
        "indexBytes": 10485760
      },
      "createdAt": "2026-05-21T12:00:00Z",
      "updatedAt": "2026-05-21T12:00:00Z",
      "staleReason": null
    },
    "results": [
      {
        "queryId": "query-123",
        "chunk": {
          "chunkHash": "chunk-hash",
          "path": "src/auth.rs",
          "pathHash": "path-hash",
          "byteRange": { "start": 0, "end": 128 },
          "lineRange": { "start": 1, "end": 8 },
          "contentHash": "content-hash",
          "language": "rust",
          "symbolHint": "oauth_refresh_token"
        },
        "score": 0.91,
        "proof": {
          "pathHash": "path-hash",
          "contentHash": "content-hash",
          "workspaceRootHash": "merkle-root",
          "generationId": "generation-123"
        },
        "proofVerified": true,
        "snippet": "pub fn oauth_refresh_token() { ... }"
      }
    ],
    "droppedResults": []
  }
}
```

Read source:

```json
{
  "method": "index/readChunk",
  "params": {
    "workspace": "/Users/pz/w/gode",
    "chunkHash": "chunk-hash",
    "offset": 0,
    "limit": 1024,
    "includeSource": true
  }
}
```

List proofs:

```json
{
  "method": "index/proofs/list",
  "params": {
    "workspace": "/Users/pz/w/gode"
  }
}
```

Behavior:

- `index/status` and all code-index methods default `workspace` to the server
  runtime workspace.
- `index/rebuild` rebuilds Merkle, chunk, embedding, and proof metadata in the
  code-index SQLite store, then emits `index/statusChanged`.
- `index/search` rejects blank queries with code `-32602`; `limit` defaults to
  5 and is clamped to 1..50.
- `index/search` returns proof-verified chunks and marks the search generation
  `stale` when the status root hash no longer matches the workspace.
- `index/readChunk` requires `includeSource: true`; without it the server
  returns code `-32602` with message `index/readChunk requires
  includeSource=true for policy-gated source reads`.
- `index/readChunk.limit` defaults to 4096 bytes and is clamped to 1..4096.
- `index/proofs/list` returns content proofs for all stored chunks in the
  current generation.

Notifications:

- `index/statusChanged` is emitted with payload
  `{ "status": CodeIndexStatusView }` after rebuilds.

### Retrieval router methods

Purpose: Let app and diagnostic clients inspect the retrieval route
decisions, outcomes, and promoted capability state recorded for a turn.

Request shape for all retrieval methods:

```json
{
  "threadId": "thread-123",
  "turnId": "turn-123",
  "limit": 10
}
```

Recommendations response:

```json
{
  "threadId": "thread-123",
  "turnId": "turn-123",
  "plans": [
    {
      "routeId": "route-123",
      "threadId": "thread-123",
      "turnId": "turn-123",
      "intent": "inspect_tool",
      "recommended": [
        {
          "mode": "discovery",
          "tool": "discovery.search",
          "query": "grep",
          "reason": "tool lookup should start from discovery",
          "confidence": "high",
          "itemId": "tool:builtin-coding-tools/grep"
        }
      ],
      "avoid": [],
      "timestamp": "2026-05-21T12:00:00Z"
    }
  ],
  "summary": {
    "text": "1 retrieval route planned.",
    "notes": ["discovery.search recommended for grep"],
    "truncated": false
  }
}
```

Metrics response:

```json
{
  "threadId": "thread-123",
  "turnId": "turn-123",
  "outcomes": [
    {
      "routeId": "route-123",
      "mode": "discovery",
      "tool": "discovery.search",
      "outcome": "useful",
      "firstUsefulPath": "discovery",
      "discoveryBeforeToolUse": true,
      "promotionBeforeToolUse": false,
      "wrongToolFamilyAttempts": 1,
      "resultCount": 3,
      "latencyMs": 7,
      "bytesReturned": 512,
      "estimatedTokensReturned": 128
    }
  ],
  "acceptedCount": 1,
  "ignoredCount": 1,
  "failedCount": 0,
  "outcomeCounts": { "useful": 1 },
  "modeCounts": { "discovery": 1 },
  "summary": {
    "text": "1 useful retrieval outcome.",
    "notes": [],
    "truncated": false
  }
}
```

Behavior:

- `retrieval/recommendations` returns `RetrievalRoutePlan` values emitted for
  the requested thread/turn.
- `retrieval/metrics` summarizes accepted, ignored, failed, and measured
  retrieval outcomes.
- `retrieval/promoted` returns promoted, reused, warm-cache, expired, and
  skipped promotion states for the turn.
- `limit` is optional and bounds returned event-derived rows.

Notifications:

- Retrieval runtime events are forwarded as `retrieval/routePlanned`,
  `retrieval/routeAccepted`, `retrieval/routeIgnored`,
  `retrieval/routeFailed`, `retrieval/resultUsed`,
  `retrieval/discoveryItemPromoted`, and `retrieval/promotionSkipped`.

### Eval report methods

Purpose: List and read local eval reports generated under
`<workspace>/evals/reports`.

List:

```json
{
  "method": "eval/reports/list",
  "params": {
    "limit": 10
  }
}
```

```json
{
  "reports": [
    {
      "id": "tool-calls-20260521-120000",
      "suiteId": "tool-calls",
      "fixtureCount": 12,
      "passed": 10,
      "failed": 2,
      "reliability": {
        "errorClassCounts": { "tool_schema": 1 },
        "retryAttempts": 1,
        "retryRecoveries": 1,
        "failureLimitStops": 0,
        "unknownErrors": 0
      },
      "generatedAt": "2026-05-21T12:00:00Z"
    }
  ]
}
```

Read:

```json
{
  "method": "eval/report/read",
  "params": {
    "reportId": "tool-calls-20260521-120000",
    "maxBytes": 65536
  }
}
```

```json
{
  "summary": {
    "id": "tool-calls-20260521-120000",
    "suiteId": "tool-calls",
    "fixtureCount": 12,
    "passed": 10,
    "failed": 2,
    "reliability": {
      "errorClassCounts": { "tool_schema": 1 },
      "retryAttempts": 1,
      "retryRecoveries": 1,
      "failureLimitStops": 0,
      "unknownErrors": 0
    },
    "generatedAt": "2026-05-21T12:00:00Z"
  },
  "markdown": "# Eval Report\n\n...",
  "truncated": false
}
```

Behavior:

- `eval/reports/list.limit` truncates the sorted report list when supplied.
- `eval/report/read.reportId` must be an id returned by
  `eval/reports/list`; absolute paths and `..` are rejected.
- `eval/report/read.maxBytes` defaults to 65536 and is capped at 262144.

Errors:

- Invalid report ids return code `-32602`.
- Report directory/list failures return code `-32000` with `data.details`.

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

## Chrome browser methods

Purpose: bridge JSON-RPC clients to a connected Manifest V3 browser extension.
The extension pairs over the same remote WebSocket as native clients (see
`remote.md`); these methods call the process-global browser bridge
(`roder_api::chrome`). Session-control methods (`status`, `enable`, `disable`,
`setMode`, `reconnect`) return a `ChromeStatus` (`connected`, `clientCount`,
`enabled`, `capabilities`, `mode`, `activeTab`, `browser`, `lastError`,
`remoteAddr`). The remaining methods forward a command frame to the extension
and await its `command/result`.

> Untrusted input: page snapshots, console output, network metadata, and
> permission records returned by the dispatching methods originate from the
> browser and are returned verbatim as opaque JSON. Clients must not treat that
> content as user or system instructions.

| Method | Params | Result |
| --- | --- | --- |
| `chrome/status` | — | `ChromeStatus` |
| `chrome/enable` | `{ mode?: "observe"\|"assist"\|"control" }` | `ChromeStatus` |
| `chrome/disable` | — | `ChromeStatus` |
| `chrome/setMode` | `{ mode }` | `ChromeStatus` |
| `chrome/reconnect` | — | `ChromeStatus` |
| `chrome/browsers/list` | — | `{ browsers: [] }` when disconnected, else bridge result |
| `chrome/tabs/list` | — | bridge result (`tabs/list`) |
| `chrome/tabs/activate` | `{ tabId }` | bridge result (`tab/activate`) |
| `chrome/tabs/navigate` | `{ tabId?, url }` | bridge result (`tab/navigate`) |
| `chrome/page/snapshot` | `{ tabId?, include? }` | bridge result (`page/snapshot`) |
| `chrome/page/action` | `{ action, ... }` | bridge result (`page/<action>`) |
| `chrome/debug/console` | `{ tabId?, limit? }` | bridge result (`debug/console/read`) |
| `chrome/debug/network` | `{ tabId?, limit? }` | bridge result (`debug/network/read`) |
| `chrome/permissions/list` | `{ origin? }` | bridge result (`permissions/get`) |
| `chrome/permissions/update` | `{ origin, perms }` | bridge result (`permissions/set`) |

Behavior:

- `chrome/enable` sets the session enabled flag and, when `mode` is supplied,
  the permission mode; an unknown mode returns `-32602`.
- `chrome/page/action` maps `action` (`click`, `type`, `keypress`, `scroll`,
  `select`, `screenshot`, `highlight`, `eval`) to the wire kind `page/<action>`
  and forwards the remaining params; unknown actions return `-32602`.
- Bridge failures map to JSON-RPC errors in the reserved `-32010..=-32015` band:
  not connected (`-32010`), disabled (`-32011`), rejected (`-32012`), timeout
  (`-32013`), disconnected (`-32014`), remote error (`-32015`).

## Streaming and Notifications

Subscribe through `LocalAppClient::subscribe_notifications()` for local clients
or the remote WebSocket notification stream for remote clients.

### Turn and item notifications

`thread/started`:

```json
{
  "thread": {
    "id": "thread-123",
    "preview": "Untitled thread",
    "modelProvider": "openai",
    "model": "gpt-5.5",
    "createdAt": 1770000000,
    "updatedAt": 1770000000,
    "status": { "type": "idle", "activeTurnId": null, "activeFlags": [] },
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
  "seq": 12,
  "eventId": "event-12",
  "threadId": "thread-123",
  "turnId": "turn-123",
  "timestamp": "2026-05-27T12:00:00Z",
  "event": {
    "type": "itemDelta",
    "itemId": "turn-123-agent-final_answer",
    "delta": {
      "type": "agentMessageText",
      "delta": "Hello",
      "phase": "final_answer"
    }
  }
}
```

`item/started`, `item/completed`, `item/agentMessage/delta`,
`item/reasoning/textDelta`, `item/reasoning/summaryPartAdded`, and
`item/reasoning/summaryTextDelta` all carry the same typed item-event envelope:
`seq`, `eventId`, `threadId`, `turnId`, `timestamp`, and `event`. The `event`
is `itemStarted`, `itemDelta`, or `itemCompleted`, and every lifecycle update
targets the same stable item id that later appears in `thread/read`.

`turn/completed` carries `threadId` and a terminal `turn` whose `status` is
`completed`, `failed`, or `interrupted`. Completed and failed turns include
`turn.usage` when provider usage was reported, including `cached_prompt_tokens`
and `cache_hit_rate`.

`thread/status/changed`:

```json
{
  "threadId": "thread-123",
  "status": { "type": "running", "activeFlags": ["approvalRequired"] }
}
```

`thread/goal/updated`:

```json
{
  "threadId": "thread-123",
  "goal": {
    "threadId": "thread-123",
    "objective": "Ship the goal parity slice",
    "status": "active",
    "tokensUsed": 1200,
    "timeUsedSeconds": 180,
    "createdAt": "2026-05-22T09:00:00Z",
    "updatedAt": "2026-05-22T09:03:00Z"
  }
}
```

`thread/goal/cleared`:

```json
{
  "threadId": "thread-123"
}
```

`thread/approvalRequested`:

```json
{
  "threadId": "thread-123",
  "turnId": "turn-123",
  "approvalId": "tool-call-123",
  "toolId": "tool-call-123",
  "toolName": "shell",
  "reason": "shell commands require approval"
}
```

Clients answer with `thread/resolve_approval`. `thread/approvalResolved`
echoes `threadId`, `turnId`, `approvalId`, `toolId`, `toolName`, and
`approved`.

`thread/userInputRequested`:

```json
{
  "threadId": "thread-123",
  "turnId": "turn-123",
  "requestId": "input-123",
  "questions": [
    {
      "id": "mode",
      "question": "Which mode?",
      "options": []
    }
  ]
}
```

Clients answer with `thread/resolve_user_input`. `thread/userInputResolved`
echoes `threadId`, `turnId`, `requestId`, and `answers`.

`thread/planExitRequested`:

```json
{
  "threadId": "thread-123",
  "turnId": "turn-123",
  "requestId": "exit-plan-123",
  "targetMode": "default",
  "planSummary": "Implement approved edits"
}
```

Clients answer with `thread/exit_plan`. `thread/planExitResolved` echoes
`threadId`, `turnId`, `requestId`, `approved`, `targetMode`, and
`resolvedMode`.

Ordering:

- `turn/started` is emitted before terminal `turn/completed`.
- A running status notification is emitted when a turn starts.
- Wait states keep `status.type` as `running` and set `activeFlags` to
  `approvalRequired`, `userInputRequired`, or `planExitRequired`.
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

### Automation notifications

Automation notifications let app and sibling clients distinguish running,
terminal, skipped, and blocked scheduled work:

```json
{
  "run": {
    "runId": "run-123",
    "automationId": "automation-123",
    "occurrenceKey": "automation-123:manual:2026-05-21T10:10:00Z",
    "state": "running",
    "scheduledFor": "2026-05-21T10:10:00Z",
    "threadId": "thread-123",
    "turnId": "turn-123",
    "taskId": "task-123"
  }
}
```

Methods:

- `automations/runStarted`
- `automations/runCompleted`
- `automations/runFailed`
- `automations/runSkipped`
- `automations/needsInput`

`automations/needsInput` is emitted in addition to `automations/runFailed` when
an automation turn is interrupted because it requested approval or user input.

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
- Dynamic workflows: `workflows/drafted`, `workflows/approvalRequested`,
  `workflows/approved`, `workflows/denied`, `workflows/queued`,
  `workflows/started`, `workflows/phaseStarted`,
  `workflows/phaseCompleted`, `workflows/agentQueued`,
  `workflows/agentStarted`, `workflows/agentCompleted`,
  `workflows/agentFailed`, `workflows/outputRecorded`,
  `workflows/checkpointRecorded`, `workflows/paused`,
  `workflows/resumed`, `workflows/stopped`, `workflows/completed`, and
  `workflows/failed`. Payloads mirror the corresponding
  `roder_api::dynamic_workflows` event structs and always include `runId`;
  child-agent and phase notifications include `agent` or `phase` snapshots.
- Discovery: `discovery/catalogBuilt`, `discovery/itemUpdated`,
  `discovery/authRequired`, `discovery/itemRead`,
  `discovery/itemPromoted`, `discovery/promotionReused`,
  `discovery/warmCacheHit`, `discovery/promotionExpired`.
- Retrieval: `retrieval/routePlanned`, `retrieval/routeAccepted`,
  `retrieval/routeIgnored`, `retrieval/routeFailed`,
  `retrieval/resultUsed`, `retrieval/discoveryItemPromoted`,
  `retrieval/promotionSkipped`.
- Skills: `skills/catalogLoaded`, `skills/configApplied`,
  `skills/activationResolved`, `skills/indexRendered`, `skills/invoked`,
  `skills/autoActivated`, `skills/skipped`.
- Media: `media/artifactCreated`, `media/artifactUpdated`,
  `media/artifactDeleted`, `media/previewReady`.
- Memory: `memory/saved`, `memory/updated`, `memory/deleted`,
  `memory/queried`, `memory/recallReady`, `memory/reembedQueued`,
  `memory/providerChanged`, `memory/observationRecorded`.
- Indexes: `search_index/statusChanged` and `index/statusChanged`.

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

- `thread/list` and `thread/read` use persisted threads first and in-memory
  protocol threads as a fallback.
- `providers/select`, `settings/set_web_search`, `settings/set_shell`,
  `settings/set_default_mode`, and `settings/set_file_backed_dynamic_context`
  persist only when the app-server instance enables user-config persistence.
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

### Roadmaps

1. Call `roadmap/list` to discover plans.
2. Call `thread/roadmap/open` with `{ "path": "roadmap/20-roadmapping-mode.md" }`.
3. Call `roadmap/read` or `roadmap/validate` to render document state.
4. Use `roadmap/task/update` with an `evidence` string before setting `checked: true`.
5. Use `roadmap/thread/attach` or `thread/attach` to connect existing threads.
6. ACP roadmap methods are not advertised; unsupported ACP roadmap requests return JSON-RPC method-not-found.

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

## Session storage backends

App-server thread and artifact methods are backend-neutral. The default local backend is JSONL; PostgreSQL can be selected by trusted process configuration (`[sessions]` or `RODER_SESSION_STORE=postgres`). Tenant id is not accepted in public thread or artifact method payloads; PostgreSQL tenant scope is injected by the configured session store.
