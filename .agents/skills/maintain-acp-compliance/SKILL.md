---
name: maintain-acp-compliance
description: Use when changing gode features, tools, sessions, permissions, MCP support, CLI entrypoints, model/config behavior, or docs in ways that could affect the Agent Client Protocol implementation, advertised ACP capabilities, or ACP client compatibility.
---

# Maintain ACP Compliance

## Principle

Treat ACP as a public contract. Any feature that changes what gode can do, what tools can run, how sessions work, or how users approve actions must be reflected in `internal/godex/acp`, its wire-level tests, and README examples before the task is complete.

## Required Workflow

1. Identify the feature delta.
   - Check whether the change affects sessions, prompts, content blocks, tools, permissions, cancellation, MCP servers, files, terminals, config, auth, models, or CLI startup.
   - Inspect the implementation path, not just the user-facing change.

2. Compare against the ACP surface.
   - Local ACP code: `internal/godex/acp/{protocol.go,types.go,server.go,transport.go}`.
   - ACP tests: `internal/godex/acp/server_test.go`.
   - CLI entrypoint: `cmd/gode/main.go` command `gode acp`.
   - User docs: README section `Agent Client Protocol`.
   - Current spec reference when needed: `https://agentclientprotocol.com/protocol/overview` and the upstream schema.

3. Update advertised capabilities conservatively.
   - Only advertise a capability after it is implemented and covered by tests.
   - If a feature is not supported through ACP, reject it with a clear JSON-RPC error instead of silently accepting or partially handling it.
   - Keep `promptCapabilities`, `mcpCapabilities`, and `sessionCapabilities` in sync with real behavior.

4. Add or update wire-level tests first.
   - Tests must send JSON-RPC messages through `Connection.HandleJSON` or `ServeStdio`, not only call private helpers.
   - Cover the new method/notification shape, success path, and at least one relevant error or unsupported path.
   - For streaming behavior, assert `session/update` payload shape and the final `session/prompt` `stopReason`.
   - For tool or permission changes, assert `tool_call`, `tool_call_update`, and `session/request_permission` payloads.

5. Update docs.
   - If clients need to know it, update README `Agent Client Protocol`.
   - Include exact JSON examples for new methods, content blocks, capabilities, or permission outcomes.
   - Keep docs aligned with advertised capabilities, not future plans.

6. Verify before finishing.
   - Run `go test ./internal/godex/acp -count=1`.
   - Run the narrow package touched by the feature.
   - Run `go test ./...` unless unrelated concurrent work makes that impossible; if blocked, report the exact package and error.
   - For CLI or transport changes, run a real `go run ./cmd/gode acp ...` stdio smoke.

## ACP Change Checklist

- New tool: ACP emits useful `tool_call`/`tool_call_update` with stable `toolCallId`, `title`, `kind`, `status`, inputs, and outputs.
- New permission flow: ACP sends `session/request_permission` and bridges the client response back to `eventbus.KindPermissionResponded`.
- New prompt content type: capability is advertised, parser accepts it, unsupported variants still fail clearly, README shows an example.
- New session behavior: method/notification names match ACP schema, cancellation returns `stopReason: "cancelled"` when appropriate.
- New MCP support: `session/new` validation, transport support, tool registration, and advertised `mcpCapabilities` all match.
- New config/model/auth behavior: `initialize` response, session defaults, and README examples remain accurate.
- New CLI startup option: `gode acp` command docs and smoke coverage still work.

## Common Failures

- Advertising optional support before it exists.
- Adding a feature to TUI/appserver but forgetting ACP clients.
- Testing private helpers but not the JSON-RPC envelope.
- Updating README with planned behavior instead of implemented behavior.
- Treating `go test ./internal/godex/acp` as enough after touching shared runner, tool, MCP, or config code.
