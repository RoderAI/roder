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

```sh
./jaeger.sh
make run
```

Then open Jaeger at <http://localhost:16686>.

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

Supported transports:

- `--listen stdio://` for JSONL over stdin/stdout.
- `--listen ws://IP:PORT` for WebSocket frames plus `/readyz` and `/healthz`.
- `--listen off` to disable the local control transport.

## Near-Term Direction

- Deepen the OpenAI/Codex provider loop with multi-turn tool result continuation.
- Add the Anthropic Claude SDK Go adapter.
- Expand MCP connection coverage and external plugin surfaces.
- Keep files small and split logic when a file starts getting large.
