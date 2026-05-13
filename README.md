# gode

Go Code: a lightweight Go-native TUI coding agent.

`gode` is intended to become a local coding agent that can talk to Codex and the Anthropic Claude SDK for Go, while also carrying its own harness for sessions, tools, workspace state, and terminal-oriented workflows.

This repository is only a tiny scaffold right now. The real implementation comes next.

## Shape

- `cmd/gode`: CLI entrypoint.
- `internal/harness`: future orchestration layer for sessions, tools, and workspace state.
- `internal/provider`: shared provider interface plus Codex and Claude placeholders.

## Try It

```sh
go run ./cmd/gode
go run ./cmd/gode version
```

## Near-Term Direction

- Add a small TUI shell.
- Wire provider adapters for Codex and Claude.
- Define the harness contracts for tool calls, patches, and session state.
- Keep files small and split logic when a file starts getting large.
