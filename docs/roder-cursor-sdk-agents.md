# Cursor SDK Agents (remote Cursor cloud agents)

Roder can create, stream, resume, and cancel **remote Cursor cloud
agents** — Cursor-hosted VMs that clone a repository, run a prompt with
the user's Cursor account, survive the caller disconnecting, and can open
pull requests — through a process-hosted TypeScript extension wrapping the
official `@cursor/sdk` (roadmap phase 95).

This is separate from the native `cursor` inference provider
(`docs/roder-cursor-provider.md`), which drives Cursor Composer models
through AgentService for normal Roder turns. This extension owns the
official-SDK and cloud-agent path that the inference provider explicitly
excluded.

## Architecture

- `examples/non-rust-extensions/cursor-sdk-agents/` is a TypeScript child
  speaking the process-extension protocol 0.2.0 over stdio (see
  `docs/roder-process-extensions.md`).
- The child provides two services, bridged by `roder-ext-process-host`
  into canonical Roder contracts:
  - `cursor-cloud` (`subagent_dispatcher` → `SubagentDispatcher`): one
    dispatch maps to one cloud agent run; status events stream into the
    subagent trace sink and the terminal `SubagentResult` carries the
    final summary plus agent/request ids, branches, and PR URLs in
    `metadata`.
  - `cursor-cloud-agent` (`task_executor` → `TaskExecutor`): background
    tasks through the standard `tasks/submit`/`tasks/get`/`tasks/cancel`
    app-server surfaces, with progress streamed into the task log.
- `@cursor/sdk` is pinned in the example's `package.json` (currently
  `1.0.18`) and loaded behind a seam; `CURSOR_SDK_FAKE=1` selects an
  in-process fake for offline tests. `tool_call` payloads are documented
  as unstable upstream, so only the stable envelope (name, status) is
  forwarded.

## Setup

```sh
cd examples/non-rust-extensions/cursor-sdk-agents
npm ci && npm run build
```

```toml
[[process_extensions]]
id = "cursor-sdk"
enabled = true
manifest = "examples/non-rust-extensions/cursor-sdk-agents/roder-extension.toml"
command = "node"
args = ["dist/src/main.js"]
cwd = "examples/non-rust-extensions/cursor-sdk-agents"
env = { CURSOR_API_KEY = "..." }
startup_timeout_ms = 15000
```

`CURSOR_API_KEY` reaches the child only through this explicit env
allowlist. The child redacts key material (`crsr_...` values, bearer
headers, the raw key) from every error, event, log line, and result.

## Dispatching a cloud agent

Task input (`tasks/submit` with `executor_id: "cursor-cloud-agent"`) and
dispatcher structured input (`SubagentRequest.inputs`) share one schema:

| Key | Type | Meaning |
| --- | --- | --- |
| `prompt` | string (required) | Prompt for the cloud agent (the dispatcher uses `SubagentRequest.prompt`). |
| `repoUrl` | string | `https://` repository URL; required when creating. |
| `startingRef` | string | Git ref the agent starts from. |
| `autoCreatePr` | boolean | Ask Cursor to open a PR with the agent's changes. |
| `model` | string | Cursor model id, e.g. `composer-2.5`. |
| `agentId` | string | Existing `bc-` cloud agent id to resume instead of creating. |
| `wait` | boolean | Task executor only. `false` = submit and return ids immediately. |

Unknown keys, non-`https` repo URLs, non-`bc-` agent ids, and
creation-only options combined with `agentId` are rejected before any SDK
call.

The completed task payload (and dispatcher result metadata) carries:

```json
{
  "agentId": "bc-...",
  "requestId": "...",
  "runId": "...",
  "status": "finished",
  "result": "final assistant text",
  "model": "composer-2.5",
  "durationMs": 12345,
  "branches": [{ "repoUrl": "...", "branch": "...", "prUrl": "..." }],
  "prUrls": ["https://github.com/.../pull/7"],
  "waited": true,
  "resumed": false
}
```

## Resume across restarts

Cloud agents keep running server-side. Persist `agentId` (`bc-` prefix)
from the payload; a later dispatch with `agentId` set reattaches through
`Agent.resume` and continues the same conversation. `wait: false` is the
fire-and-forget shape: submit, persist the ids, resume later.

## Permission posture

- Dispatch through model-facing tool execution (the task tool / workflow
  executors that wrap registry dispatchers) stays behind Roder's normal
  tool approval policy.
- `tasks/submit` is a client-initiated surface, equivalent to other task
  executors: the caller is the authority.
- A remote cloud agent acts with the Cursor account's authority on the
  configured repository — it can push branches and open PRs. Point
  dispatch inputs only at repositories you intend the agent to change.

## Verification

Offline (no network, no key — fake SDK and fixture children):

```sh
cargo test -p roder-api --test process_extension_protocol
cargo test -p roder-ext-process-host
(cd examples/non-rust-extensions/cursor-sdk-agents && npm test)
cargo test -p roder-app-server --features e2e-tests --test process_extension_cursor_sdk
```

Live (opt-in; consumes Cursor usage and runs a real cloud VM):

```sh
RODER_CURSOR_SDK_LIVE=1 \
CURSOR_API_KEY=... \
CURSOR_SDK_LIVE_REPO_URL="https://github.com/<org>/<disposable-repo>" \
cargo test -p roder-app-server --features e2e-tests \
  --test process_extension_cursor_sdk -- --ignored --nocapture
```

The live check dispatches one cloud agent with a non-destructive prompt
(`autoCreatePr: false`), then resumes it by the persisted `bc-` id.

## Troubleshooting

- `did not initialize within …ms`: run `npm ci && npm run build` in the
  example directory; the host needs `dist/src/main.js` to exist and `node`
  on `PATH`.
- `CURSOR_API_KEY is not configured`: add the key to the process-extension
  `env` allowlist; the child never reads the host environment implicitly.
- `unknown agent bc-…` on resume: the agent was archived/deleted or
  belongs to a different Cursor account.
- `unsupported input key …`: the input schema is strict; see the table
  above.
