# Process-Hosted Extensions

Roder can host extensions written in any language as child processes. A
process extension registers ordinary extension services (inference
engines, event sinks, subagent dispatchers, task executors) through a
manifest and speaks newline-delimited JSON-RPC 2.0 over stdio. App-server
clients cannot tell a process-hosted service from a native Rust one except
through extension metadata.

Reference implementations:

- Python: the OpenAI-compatible chat-completions provider POC under
  `examples/non-rust-extensions/python-chat-completions/` (inference
  engine + event sink).
- TypeScript: the Cursor SDK remote-agents extension under
  `examples/non-rust-extensions/cursor-sdk-agents/` (subagent dispatcher +
  task executor; see `docs/roder-cursor-sdk-agents.md`).

## Configuration

```toml
[[process_extensions]]
id = "python-chat-completions"
enabled = true
manifest = "examples/non-rust-extensions/python-chat-completions/roder-extension.toml"
command = "python3"
args = ["-m", "roder_python_chat_provider"]
cwd = "examples/non-rust-extensions/python-chat-completions"
env = { PYTHONUNBUFFERED = "1", PYTHONPATH = "src" }
startup_timeout_ms = 10000
event_filter = { kinds = ["turn.", "inference."] }
```

- `manifest` (relative paths resolve against the workspace) points at the
  extension manifest TOML — the registry source of truth for the extension
  id, version, API requirement, provided services, and capabilities.
- `env` is an explicit allowlist. The host clears the child environment and
  forwards only these entries (plus `PATH`). Secrets reach the child only
  through entries you configure, e.g.
  `env = { PY_CHAT_COMPLETIONS_API_KEY = "..." }`, and are never echoed in
  manifests, errors, or events.
- `event_filter.kinds` are canonical event-kind prefixes the child receives
  (`turn.`, `inference.`, …). Empty = no events forwarded.
- Disabled entries are skipped. Enabled entries with missing or invalid
  manifests fail registry construction with a precise error.

## Protocol

Newline-delimited JSON-RPC 2.0 over stdio. The host owns request ids; child
diagnostics belong on stderr. Canonical DTOs live in
`roder_api::process_extension` and the protocol version is `0.2.0`
(bumped from 0.1.0 when subagent dispatcher and task executor services
were added; children echo the version, so a stale child fails closed).

| Method | Direction | Purpose |
| --- | --- | --- |
| `extension/initialize` | host → child request | API/protocol versions, granted capabilities, redacted config, event filter. The child echoes its id, services, and an FNV-1a checksum of its manifest TOML; any mismatch with the configured manifest fails closed. |
| `inference/listModels` | host → child request | Canonical `ModelDescriptor` values. |
| `inference/streamTurn` | host → child request | Canonical `AgentInferenceRequest` plus thread/turn ids and a host-chosen `streamId`. The child acks `{streamId}` then streams events. |
| `inference/event` | child → host notification | `{streamId, event}` with canonical `InferenceEvent` payloads until `Completed` or `Failed`. |
| `subagents/definitions` | host → child request | Canonical `SubagentDefinition` values for a dispatcher service. |
| `subagents/dispatch` | host → child request | Parent thread/turn ids plus a canonical `SubagentRequest` and a host-chosen `dispatchId`. The child acks `{dispatchId}` then streams events. |
| `subagents/event` | child → host notification | `{dispatchId, event}` where event is `status` (redacted progress, forwarded to subagent trace sinks), or terminal `completed` (canonical `SubagentResult`) / `failed`. |
| `subagents/cancel` | host → child request | Cancel an in-flight dispatch (host timeout or caller cancellation). |
| `tasks/spec` | host → child request | Canonical `TaskSpec` for a task executor service. |
| `tasks/execute` | host → child request | Task id/provenance plus executor input and a host-chosen `executionId`. The child acks `{executionId}` then streams events. |
| `tasks/event` | child → host notification | `{executionId, event}` where event is `output` (forwarded into the task output sink), or terminal `completed` (canonical `TaskExecutionResult`) / `failed`. |
| `tasks/cancel` | host → child request | Cancel an in-flight execution (sent automatically when the host-side task is aborted). |
| `events/handle` | host → child notification | Filtered canonical `EventEnvelope` values. |
| `extension/event` | child → host notification | Typed extension-owned events (`extensionId`, `eventKind`, `schemaVersion`, redacted `payload`). |
| `extension/shutdown` | host → child request | Graceful shutdown before the process is killed. |

## Runtime behavior

- Children spawn lazily on the first service use, with a startup timeout and
  initialize-echo validation (id, protocol version, services, manifest
  checksum) that fails closed.
- Stdout lines are size-capped; non-JSON lines are dropped with a stderr
  diagnostic; a child exit fails pending requests and active streams
  (inference, dispatch, and task executions) with explicit errors.
- Event delivery to sinks is bounded and non-blocking: each registered sink
  gets its own queue and worker, slow or broken sinks surface redacted
  `extension.event_sink_failed` events (never re-dispatched to sinks, so no
  loops), and turns complete even when a sink hangs.
- Dispatcher definitions and task specs are fetched from the child in the
  background and cached for the synchronous registry accessors; a
  deterministic placeholder spec is served only before the first fetch
  lands.
- Subagent dispatches honor `SubagentRequest.timeout_seconds` (default 30
  minutes); on timeout the host sends `subagents/cancel` and fails the
  dispatch. Aborted host-side tasks notify the child via `tasks/cancel` so
  remote work is not silently orphaned.

## Python POC

The example maps Roder requests to OpenAI-compatible `/chat/completions`
with `stream: true`, emits canonical `MessageDelta`/`ToolCallCompleted`/
`Usage`/`ProviderMetadata`/`Completed`/`Failed` events, and records received
Roder event kinds (names and counts only — never prompts or secrets) into
provider metadata. See its README for configuration and the live smoke gate.

## Local verification

```sh
cargo test -p roder-api --test process_extension_protocol
cargo test -p roder-ext-process-host
cargo test -p roder-core --test process_extension_events
cargo test -p roder-config process_extensions
cargo test -p roder-extension-host process_extensions
cargo test -p roder-app-server --features e2e-tests --test process_extension_python_provider
(cd examples/non-rust-extensions/python-chat-completions && python3 -m unittest discover -s tests)
```

All tests run offline with fake children and fake HTTP servers. The live
check is opt-in:

```sh
RODER_PROCESS_EXT_LIVE=1 \
PY_CHAT_COMPLETIONS_API_KEY="$OPENAI_API_KEY" \
PY_CHAT_COMPLETIONS_BASE_URL="https://api.openai.com/v1" \
PY_CHAT_COMPLETIONS_MODEL="gpt-5.5" \
cargo test -p roder-app-server --features e2e-tests \
  --test process_extension_python_provider -- --ignored --nocapture
```

## Troubleshooting

- `did not initialize within …ms`: the command failed to start or the child
  never answered `extension/initialize`. Run the command manually from the
  configured `cwd`; child stderr appears in host diagnostics prefixed with
  `[process-ext <id>]`.
- `echoed manifest checksum … different manifest`: the child package ships a
  different manifest than the one configured. Point `manifest` (and the
  child's `RODER_EXTENSION_MANIFEST`, if set) at the same file.
- `dropped non-JSON stdout line`: the child wrote logs to stdout. Protocol
  frames only on stdout; logs go to stderr.
