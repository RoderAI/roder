# gode

Go Code: a Go-native TUI coding agent and event-driven agent harness.

`gode` is built around `godex`, an internal harness for coding-agent sessions, tools, workspace state, MCPs, context management, provider adapters, and event subscriptions. The goal is to provide a solid open-source base that ML labs can use to train, evaluate, and run their own coding agents.

## Shape

- `cmd/gode`: CLI entrypoint.
- `internal/godex`: event bus, JSONL journal, runner, provider interface, MCP manager, and tool registry.
- `internal/godex/tools/builtin`: broad initial coding tool set.
- `internal/tui`: Bubble Tea UI with composable Lip Gloss components and view models.

Every meaningful state transition flows through the `godex` event bus so the TUI, plugins, logs, tests, and future RL/replay systems can subscribe to the same stream.

## Try It

```sh
go run ./cmd/gode version
go run ./cmd/gode
make run
make ask PROMPT="summarize this repo"
```

## Near-Term Direction

- Deepen the OpenAI/Codex provider loop with multi-turn tool result continuation.
- Add the Anthropic Claude SDK Go adapter.
- Expand MCP connection coverage and external plugin surfaces.
- Keep files small and split logic when a file starts getting large.
