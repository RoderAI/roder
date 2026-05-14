# gode

Go Code: a Go-native TUI coding agent and event-driven agent harness.

`gode` is built around `godex`, an internal harness for coding-agent sessions, tools, workspace state, MCPs, context management, provider adapters, and event subscriptions. The goal is to provide a solid open-source base that ML labs can use to train, evaluate, and run their own coding agents.

## Alpha Status

This is alpha software and will change rapidly as it materializes its shape. For the time being, expect no API guarantees: packages, commands, configuration, event formats, and integration points may move, break, or disappear without notice.

## Shape

- `cmd/gode`: CLI entrypoint.
- `internal/godex`: event bus, JSONL journal, runner, provider interface, MCP manager, and tool registry.
- `internal/godex/appserver`: Codex-style app-server protocol, request handling, and transports for desktop control.
- `internal/godex/tools/builtin`: broad initial coding tool set.
- `internal/tui`: Bubble Tea UI with composable Lip Gloss components and view models.

Every meaningful state transition flows through the `godex` event bus so the TUI, plugins, logs, tests, and future RL/replay systems can subscribe to the same stream.

## Try It

```sh
go run ./cmd/gode version
go run ./cmd/gode
go run ./cmd/gode resume
go run ./cmd/gode acp --provider mock --auto-approve
go run ./cmd/gode app-server --listen ws://127.0.0.1:0
make run
make ask PROMPT="summarize this repo"
```

`make run` enables local OpenTelemetry tracing by default and exports OTLP/gRPC spans to `localhost:4317`.
`gode resume` opens a compact inline session picker. Type to search, press `tab` to toggle between sessions from the current workspace and all sessions, then press `enter` to resume.

```sh
./jaeger.sh
make run
```

Then open Jaeger at <http://localhost:16686>.

## Custom Models

Add custom models to `$HOME/.gode/config.toml` or a workspace `.gode.toml` with `[model.<local-id>]` tables. The table key is the model ID you select in the TUI, CLI, and `gode models`; the `model` field is the upstream model name sent to the provider.

OpenAI-compatible Chat Completions endpoints:

```toml
[model.deepseek-chat]
type = "chat_completions"
provider = "deepseek"
model = "deepseek-chat"
display_name = "DeepSeek Chat"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
context_window = 128000
default_reasoning = "none"
reasoning_efforts = ["none"]
edit_tool = "edit"

[model.kimi-k2-6]
type = "chat_completions"
provider = "moonshot"
model = "kimi-k2.6"
display_name = "Kimi K2.6"
base_url = "https://api.moonshot.ai/v1"
api_key_env = "MOONSHOT_API_KEY"
context_window = 262144
default_reasoning = "none"
reasoning_efforts = ["none"]
```

Responses-compatible and Anthropic-compatible routers:

```toml
[model.my-responses-model]
type = "responses"
provider = "openai-compatible"
model = "my-responses-model"
display_name = "My Responses Model"
base_url = "https://router.example.com/v1"
api_key = "env:MY_RESPONSES_API_KEY"
context_window = 200000
edit_tool = "patch"

[model.my-claude-router]
type = "anthropic"
provider = "anthropic-compatible"
model = "claude-compatible-model"
display_name = "My Claude Router"
base_url = "https://router.example.com"
api_key_env = "ROUTER_API_KEY"
context_window = 200000
default_reasoning = "medium"
reasoning_efforts = ["low", "medium", "high"]
edit_tool = "edit"
```

`edit_tool` controls which write primitive is loaded and exposed to the model. Use `patch` for GPT-style models so they only get `apply_patch`; use `edit` for non-GPT models so they get `write_file`, `edit`, and `multi_edit` instead. When omitted, models whose upstream name starts with `gpt` default to `patch`; all other models default to `edit`.

Inspect the active catalog with:

```sh
gode models
```

Optional live Chat Completions smoke tests are skipped by default:

```sh
GODE_LIVE_CHAT_COMPLETIONS=1 \
GODE_MODEL=deepseek-chat \
GODE_LIVE_CHAT_COMPLETIONS_BASE_URL=https://api.deepseek.com/v1 \
GODE_LIVE_CHAT_COMPLETIONS_API_KEY="$DEEPSEEK_API_KEY" \
go test ./internal/godex/provider -run TestChatCompletionsLive -v
```

## Shell Tooling

The `shell` tool runs POSIX shell strings through the embedded `mvdan.cc/sh/v3` runner instead of spawning `/bin/sh -lc`. Standard shell behavior such as variables, pipelines, redirections, and exit statuses is handled in-process, while unknown external commands still fall through to OS command lookup and return command-not-found failures.

Initial in-process builtins are available before PATH lookup:

- `jq`: a small `gojq`-backed JSON builtin with `-r`, stdin input, and file input.
- `gode_read_file path [start_line] [limit]`: reads text files through the same ranged read path as the `read_file` tool.
- `gode_list_files [path]`: lists sorted direct children through the `list_files` tool.
- `gode_search_files query`: searches workspace text files through the `search_files` tool.
- `gode_apply_patch`: in patch-mode sessions, reads a patch from stdin and applies it through the existing `apply_patch` tool.

The app-server keeps direct process execution for array commands such as `["/bin/echo","hi"]`. Non-streaming shell invocations like `["sh","-c","jq -r .name file.json"]` and `["/bin/sh","-lc","gode_list_files ."]` route through the embedded shell runner so app-server clients get the same builtins as agent shell tool calls.

## Homebrew Release

The formula builds `gode` from source, so local installs do not need a signed binary artifact.

Create a local Homebrew release from a clean working tree:

```sh
VERSION=0.1.0 make release-brew
brew install --build-from-source ./Formula/gode.rb
```

That creates `dist/gode-v0.1.0.tar.gz`, computes its checksum, and writes `Formula/gode.rb` with a `file://` URL for local testing. To publish a formula update that points at the git tag instead:

```sh
VERSION=0.1.0 PUBLISH=1 make release-brew
```

`PUBLISH=1` creates tag `v0.1.0`, rewrites the formula to use the git tag and revision, commits `Formula/gode.rb`, and pushes the tag plus the current branch to `origin`.

## Shell And Builtins

The `shell` tool parses commands with `mvdan.cc/sh/v3` in POSIX mode instead of wrapping every command in `/bin/sh -lc`. Pipelines, redirections, variable assignments, command substitutions, and shell functions are interpreted by the embedded runner. External commands still run through the OS path unless a caller explicitly disables them.

Gode registers a small in-process builtin set before OS command lookup:

- `jq`: JSON queries from stdin or file args, including `-r` raw string output.
- `gode_read_file path [start_line] [limit]`: read focused file ranges through the existing `read_file` tool.
- `gode_list_files [path]`: list sorted direct children through the existing `list_files` tool.
- `gode_search_files query`: search text files through the existing `search_files` tool.
- `gode_apply_patch`: in patch-mode sessions, read a patch from stdin and apply it through the existing `apply_patch` tool.

`gode app-server` routes non-TTY `["sh", "-c", "..."]` and `["/bin/sh", "-lc", "..."]` command invocations through the same embedded shell runner when stdin/stdout streaming is not requested. Direct array commands and streamed app-server commands still use normal OS process execution.

## Agent Client Protocol

`gode acp` runs gode as an [Agent Client Protocol](https://agentclientprotocol.com/protocol/overview) agent over stdio. It speaks JSON-RPC 2.0 with one JSON message per line on stdin/stdout, which is the transport expected by ACP clients.

```sh
go run ./cmd/gode acp \
  --workspace "$PWD" \
  --data-dir "$HOME/.gode" \
  --provider openai \
  --model gpt-5.5
```

For offline testing, use the deterministic mock provider:

```sh
go run ./cmd/gode acp --provider mock --auto-approve
```

The ACP server advertises only capabilities that gode currently supports:

- `session/new`, `session/prompt`, `session/cancel`, and `session/update`.
- Optional `session/list` and `session/close`.
- Text and `resource_link` prompt blocks. Images, audio, and embedded resources are rejected unless those capabilities are added later.
- Stdio MCP servers passed in `session/new`. HTTP and SSE MCP transports are not advertised.
- Tool progress through `tool_call` and `tool_call_update` session updates.
- Permission requests through `session/request_permission` when a tool needs user approval.

### Minimal Client Exchange

Start by sending `initialize`:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientInfo":{"name":"example-client","version":"0.1.0"}}}
```

The response includes the negotiated protocol version, agent info, and the supported capability surface:

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"gode","title":"Gode","version":"dev"},"agentCapabilities":{"loadSession":false,"mcpCapabilities":{"http":false,"sse":false},"promptCapabilities":{"image":false,"audio":false,"embeddedContext":false},"sessionCapabilities":{"list":{},"close":{}}},"authMethods":[]}}
```

Create a session with an absolute working directory and an MCP server list, even if it is empty:

```json
{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/absolute/path/to/workspace","mcpServers":[]}}
```

Send a prompt using ACP content blocks:

```json
{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"SESSION_ID","prompt":[{"type":"text","text":"summarize this repo"},{"type":"resource_link","name":"README.md","uri":"file:///absolute/path/to/workspace/README.md"}]}}
```

While the turn runs, gode emits `session/update` notifications. A normal assistant delta looks like this:

```json
{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"SESSION_ID","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"..."}}}}
```

When the turn completes, the original `session/prompt` request receives a stop reason:

```json
{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}
```

To cancel an active turn, send a notification:

```json
{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"SESSION_ID"}}
```

The active prompt request will return:

```json
{"jsonrpc":"2.0","id":3,"result":{"stopReason":"cancelled"}}
```

### MCP Servers

ACP `session/new` can attach stdio MCP servers. `command` must be an absolute path:

```json
{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/absolute/path/to/workspace","mcpServers":[{"name":"tools","command":"/absolute/path/to/mcp-server","args":["--stdio"],"env":[{"name":"EXAMPLE","value":"1"}]}]}}
```

Tools exposed by that MCP server are registered into gode's tool registry as `mcp.<server-name>.<tool-name>`.

### Permissions

If a tool needs approval, gode sends a JSON-RPC request to the client:

```json
{"jsonrpc":"2.0","id":"permission-...","method":"session/request_permission","params":{"sessionId":"SESSION_ID","toolCall":{"toolCallId":"call-1","title":"shell.exec","kind":"execute","status":"pending"},"options":[{"optionId":"allow_once","name":"Allow once","kind":"allow_once"},{"optionId":"reject_once","name":"Reject","kind":"reject_once"}]}}
```

Reply with the selected outcome:

```json
{"jsonrpc":"2.0","id":"permission-...","result":{"outcome":{"outcome":"selected","optionId":"allow_once"}}}
```

## App Server

`gode app-server` mirrors Codex's app-server naming and wire style: JSON-RPC-shaped messages without the `jsonrpc` field, an `initialize` handshake before requests, and Codex-like method names such as `thread/start`, `turn/start`, `turn/steer`, `fs/readFile`, and `command/exec`. Use `turn/steer` with `threadId`, `expectedTurnId`, and text `input` to add steering instructions to the active turn.

The `initialize` response includes `capabilities.turnInput` so clients can discover supported `turn/start` input blocks. `turn/start` accepts text, remote images, local images, and local files:

```json
{"id":2,"method":"turn/start","params":{"threadId":"THREAD_ID","input":[{"type":"text","text":"Review these attachments"},{"type":"local_file","path":"/Users/pz/Desktop/spec.md"},{"type":"local_file","path":"/Users/pz/Desktop/screenshot.png"}]}}
```

Local images are encoded as model image input. Text files are included in the prompt up to the advertised `maxLocalFileBytes`; binary files are represented by metadata so the agent can still reason about the attachment path.

Supported transports:

- `--listen stdio://` for JSONL over stdin/stdout.
- `--listen ws://IP:PORT` for WebSocket frames plus `/readyz` and `/healthz`.
- `--listen off` to disable the local control transport.

Remote WebSocket mode is explicit and token-authenticated:

```sh
gode app-server --remote
gode app-server --remote --listen ws://0.0.0.0:0
gode app-server --remote --listen ws://100.x.y.z:0 --auth-token env:GODE_REMOTE_TOKEN
```

Remote mode defaults to `ws://0.0.0.0:0`, generates a high-entropy bearer token, stores only its hash in memory, and prints connect URLs plus a terminal QR code to stderr. Remote clients authenticate with `Authorization: Bearer <token>` or the WebSocket subprotocol pair `gode.remote.v1, bearer.<token>`. The token is not accepted in query parameters.

Inside the TUI, run `/remote` or open `ctrl+p` -> `Remote Control` to start and stop the same remote app-server sidecar. The panel shows connection URLs, a token preview, QR pairing, auth hints, connected-client count, and a LAN-without-TLS warning when relevant.

Remote connection patterns:

- Same-network phone: start `gode app-server --remote --listen ws://0.0.0.0:0`, scan the QR, and use one of the rendered `192.168.x.x` or `10.x.x.x` URLs. The QR prefers these private LAN URLs by default.
- Tailscale: bind explicitly with `--listen ws://100.x.y.z:0` when you want the QR to advertise a Tailscale address.
- Native clients: set `Authorization: Bearer <token>` during the WebSocket handshake.
- Browser-constrained clients: request subprotocols `gode.remote.v1` and `bearer.<token>` instead of sending a custom header.

## Near-Term Direction

- Deepen the OpenAI/Codex provider loop with multi-turn tool result continuation.
- Add the Anthropic Claude SDK Go adapter.
- Expand MCP connection coverage and external plugin surfaces.
- Keep files small and split logic when a file starts getting large.
