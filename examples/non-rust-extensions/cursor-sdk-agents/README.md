# Cursor SDK Agents (process-hosted TypeScript extension)

Roder process extension (roadmap phase 93) that wraps the official Cursor
TypeScript SDK so Roder can create, stream, resume, and cancel **remote
Cursor cloud agents** through canonical Roder surfaces:

- `cursor-cloud` — a `subagent_dispatcher`: one dispatch maps to one cloud
  agent run and resolves with a canonical `SubagentResult` (final summary,
  `bc-` agent id, request id, branches, PR URLs).
- `cursor-cloud-agent` — a `task_executor` for background tasks: create or
  resume (`agentId`) a cloud agent, stream progress into the task log, and
  return a structured payload. `wait: false` submits and returns ids
  immediately; cloud runs survive the caller disconnecting.

The extension speaks the Roder process-extension protocol 0.2.0
(newline-delimited JSON-RPC over stdio); see
`docs/roder-process-extensions.md`.

## SDK dependency

`@cursor/sdk` is pinned in `package.json` (currently `1.0.18`, checked
2026-06-12). The SDK is loaded behind a seam (`src/sdk.ts`):

- production: dynamic import of `@cursor/sdk`;
- `CURSOR_SDK_FAKE=1`: the in-process fake (`src/fake.ts`) used by the
  offline node tests and the Rust app-server e2e — no network, no key.

`tool_call` args/results are documented as unstable by Cursor; only the
stable envelope (name, status) is forwarded into progress events.

## Setup

```sh
cd examples/non-rust-extensions/cursor-sdk-agents
npm ci          # or npm install on first checkout
npm run build   # emits dist/
npm test        # offline tests against the fake SDK
```

## Roder configuration

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

`CURSOR_API_KEY` reaches the child only through this explicit allowlist
and is redacted from every error, event, and result the child emits.

## Inputs

Task input (`tasks/submit` with executor `cursor-cloud-agent`) and
dispatcher structured input (`SubagentRequest.inputs`):

| Key | Type | Meaning |
| --- | --- | --- |
| `prompt` | string (required) | Prompt for the cloud agent (the dispatcher uses `SubagentRequest.prompt`). |
| `repoUrl` | string | `https://` repository URL; required when creating. |
| `startingRef` | string | Git ref to start from. |
| `autoCreatePr` | boolean | Ask Cursor to open a PR with the agent's changes. |
| `model` | string | Cursor model id, e.g. `composer-2.5`. |
| `agentId` | string | Existing `bc-` cloud agent id to resume instead of creating. |
| `wait` | boolean | Task executor only. `false` = submit and return ids immediately. |

Unknown keys are rejected before any SDK call. `repoUrl`/`startingRef`/
`autoCreatePr` are creation-only and rejected together with `agentId`.

## Security posture

A remote Cursor cloud agent clones the configured repository and acts with
the Cursor account's authority: it can push branches and (with
`autoCreatePr`) open pull requests. Treat dispatch inputs as
side-effecting; point them only at repositories you intend the agent to
change.

## Live verification (opt-in)

Normal tests never touch the network. The live smoke is gated:

```sh
RODER_CURSOR_SDK_LIVE=1 \
CURSOR_API_KEY=... \
CURSOR_SDK_LIVE_REPO_URL="https://github.com/<org>/<disposable-repo>" \
cargo test -p roder-app-server --features e2e-tests \
  --test process_extension_cursor_sdk -- --ignored --nocapture
```
