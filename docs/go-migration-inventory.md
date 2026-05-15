# Go Migration Inventory

This document is an inventory of the existing Go implementation of Roder (currently housed in `internal/godex/` and `internal/tui/`) to assist with the Rust rewrite.

## Provider implementations
- `internal/godex/provider/openai.go`
- `internal/godex/provider/anthropic.go`
- `internal/godex/provider/gemini.go`
- `internal/godex/provider/chat_completions.go`
- `internal/godex/provider/mock.go`
- Configs: `internal/godex/provider/model_config.go`, `internal/godex/config.go`
- Streaming/items: `internal/godex/provider/*_stream.go`, `*_items.go`

## Agent/Core runtime behavior
- `internal/godex/agent/runner.go` (main tool loop, steer logic, streams)
- `internal/godex/agent/instructions.go`
- `internal/godex/agent/compaction.go`

## Memory implementation
- `internal/godex/memory/store.go`
- `internal/godex/memory/recall.go`
- `internal/godex/memory/vector.go`
- `internal/godex/memory/embedding.go`

## Context persistence / Session
- `internal/godex/session/store.go`
- `internal/godex/session/turn.go`
- `internal/godex/session/items.go`
- `internal/godex/session/repair.go`
- `internal/godex/contextpack/`
- `internal/godex/contextwindow/`

## App server / JRPC / Events
- `internal/godex/appserver/server.go`
- `internal/godex/appserver/protocol.go`
- `internal/godex/appserver/transport.go`
- `internal/godex/eventbus/bus.go`

## Tool execution behavior
- `internal/godex/tools/registry.go`
- Builtins: `internal/godex/tools/builtin/` (files, edit, patch, shell, search, git, mcp, lsp, etc.)

## CLI / TUI behavior
- `cmd/gode/main.go`, `serve.go`, `session_cmd.go`
- `internal/tui/` (Model, view, update, remote control, diff view, components)
- `internal/tui/remote/server.go` (TUI connecting to appserver)

## Testing and Fixtures
- Tests exist alongside implementations (`*_test.go`).
- Providers have fixtures (e.g., `internal/godex/provider/*_test.go`).
