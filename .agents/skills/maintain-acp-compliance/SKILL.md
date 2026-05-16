---
name: maintain-acp-compliance
description: Use when changing Roder Rust features, tools, sessions, permissions, MCP support, CLI entrypoints, model/config behavior, app-server behavior, or docs in ways that could affect the Agent Client Protocol implementation, advertised ACP capabilities, or ACP client compatibility.
---

# Maintain ACP Compliance

## Principle

Treat ACP as a public contract for the Roder Rust app. Any feature that changes what `roder` can do, what tools can run, how sessions work, how app-server clients observe state, or how users approve actions must be reflected in the Roder ACP/protocol surface, its wire-level tests, and docs before the task is complete.

Do not satisfy new ACP work by updating only the legacy Go `internal/godex/acp` package. If the Rust Roder ACP surface is missing or incomplete, implement or extend it in the Roder crates as part of the change.

## Required Workflow

1. Identify the feature delta.
   - Check whether the change affects sessions, prompts, content blocks, tools, permissions, cancellation, MCP servers, files, terminals, config, auth, models, or CLI startup.
   - Inspect the Roder Rust implementation path, not just the user-facing change.

2. Compare against the ACP surface.
   - Roder protocol crate: `crates/roder-protocol/src/lib.rs`.
   - Roder app-server: `crates/roder-app-server/src/server.rs`, `crates/roder-app-server/src/client.rs`, and `crates/roder-app-server/tests/e2e.rs`.
   - Roder runtime/events: `crates/roder-core/src/runtime.rs`, `crates/roder-core/src/tool_execution.rs`, and `crates/roder-api/src/events.rs`.
   - Roder CLI entrypoint: `crates/roder-cli/src/main.rs`; add or update a `roder acp` entrypoint when ACP behavior is expected from the Rust app.
   - Roder docs: README `Agent Client Protocol` section or `docs/` Roder ACP documentation.
   - Legacy Go ACP code may be useful as a reference only: `internal/godex/acp/{protocol.go,types.go,server.go,transport.go}`.
   - Current spec reference when needed: `https://agentclientprotocol.com/protocol/overview` and the upstream schema.

3. Update advertised capabilities conservatively.
   - Only advertise a capability after it is implemented and covered by tests.
   - If a feature is not supported through ACP, reject it with a clear JSON-RPC error instead of silently accepting or partially handling it.
   - Keep `promptCapabilities`, `mcpCapabilities`, and `sessionCapabilities` in sync with real behavior.

4. Add or update wire-level tests first.
   - Tests must send JSON-RPC or app-server protocol messages through the public Roder transport/client boundary, not only call private helpers.
   - Cover the new method/notification shape, success path, and at least one relevant error or unsupported path.
   - For streaming behavior, assert `session/update` payload shape and the final `session/prompt` `stopReason`.
   - For tool or permission changes, assert `tool_call`, `tool_call_update`, and `session/request_permission` payloads.

5. Update docs.
   - If clients need to know it, update README `Agent Client Protocol`.
   - Include exact JSON examples for new methods, content blocks, capabilities, or permission outcomes.
   - Keep docs aligned with advertised capabilities, not future plans.

6. Verify before finishing.
   - Run the narrow Roder Rust tests for the touched crate, such as `cargo test -p roder-protocol`, `cargo test -p roder-app-server`, or `cargo test -p roder-core`.
   - Run `cargo test -p roder-app-server --test e2e` when app-server or client-visible event behavior changes.
   - Run `cargo test --workspace` unless unrelated concurrent work makes that impossible; if blocked, report the exact package and error.
   - For CLI or transport changes, run a real `cargo run -p roder-cli --bin roder -- acp ...` stdio smoke once the `roder acp` entrypoint exists.
   - Run legacy `go test ./internal/godex/acp -count=1` only when intentionally touching the legacy Go ACP bridge.

## ACP Change Checklist

- New tool: ACP emits useful `tool_call`/`tool_call_update` with stable `toolCallId`, `title`, `kind`, `status`, inputs, and outputs.
- New permission flow: ACP sends `session/request_permission` and bridges the client response back to Roder's canonical approval/session event flow.
- New prompt content type: capability is advertised, parser accepts it, unsupported variants still fail clearly, README shows an example.
- New session behavior: method/notification names match ACP schema, cancellation returns `stopReason: "cancelled"` when appropriate.
- New MCP support: `session/new` validation, transport support, tool registration, and advertised `mcpCapabilities` all match.
- New config/model/auth behavior: `initialize` response, session defaults, and README examples remain accurate.
- New CLI startup option: `roder acp` command docs and smoke coverage exist and remain accurate for the Rust app.

## Common Failures

- Advertising optional support before it exists.
- Adding a feature to Roder TUI/app-server/runtime but forgetting ACP clients.
- Testing private helpers but not the JSON-RPC envelope.
- Updating README with planned behavior instead of implemented behavior.
- Treating the legacy Go ACP tests as enough after touching Roder runtime, tool, MCP, app-server, protocol, or config code.
