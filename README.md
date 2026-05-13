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

## App Server

`gode app-server` mirrors Codex's app-server naming and wire style: JSON-RPC-shaped messages without the `jsonrpc` field, an `initialize` handshake before requests, and Codex-like method names such as `thread/start`, `turn/start`, `fs/readFile`, and `command/exec`.

Supported transports:

- `--listen stdio://` for JSONL over stdin/stdout.
- `--listen ws://IP:PORT` for WebSocket frames plus `/readyz` and `/healthz`.
- `--listen off` to disable the local control transport.

## Near-Term Direction

- Deepen the OpenAI/Codex provider loop with multi-turn tool result continuation.
- Add the Anthropic Claude SDK Go adapter.
- Expand MCP connection coverage and external plugin surfaces.
- Keep files small and split logic when a file starts getting large.
