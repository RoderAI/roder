# External tools in a hosted browser

Roder owns the model loop. The application advertises `externalTools` on its
thread and owns how those tools affect its UI. This contract adds authenticated
execution ownership; it does not add selectors, JavaScript evaluation, or a
second agent runtime.

After starting or reading an external-tool thread, call `tools/bind_executor`
with its `threadId`. The hosted gateway requires read and write scopes and
verifies the thread in the caller's tenant runtime. It returns `executor` with
`threadId`, an opaque `leaseId`, increasing `generation`, and `contractVersion: 1`.
One authenticated WebSocket connection owns a thread. Another connection gets
`executor_busy`. Explicit `takeover: true` revokes the old executor and cancels
its pending requests; it never transfers their effects to the new executor.

Bind before `turn/start`. Only the bound connection receives
`thread/toolExecutionRequested`, including the executor and thread/turn/request
coordinate. Resolve with `tools/resolve`, including that executor, `turnId`,
`requestId`, `output`, and `isError`. A wrong connection, lease, turn, or terminal
request is rejected without consuming the owner's pending request.

`tools/unbind_executor` accepts the executor. Disconnect, authorization loss,
unbind, takeover, turn interruption, and tool timeout terminate pending work.
The gateway emits `tools/executorRevoked` with the revoked executor and reason.
Applications must abort their callbacks when ownership ends and check the
callback's signal immediately before each effect. Cancellation cannot reverse
an already committed application transaction.

After reconnect, bind anew and read fresh page state. `tools/execution_read`
accepts the current executor plus a `requestId`; its `execution` is metadata or
null. Metadata contains thread/turn/request/tool identity, state and `isError`,
never tool arguments or output bodies. Terminal metadata is bounded to one hour
and 4096 entries per tenant runtime. It is ephemeral, not a durable receipt or
proof that an interrupted browser operation had no effect. Resolve uncertain
application effects through the application's authoritative readback before
proposing another edit. Historical transcript items must never execute tools.

Use `ExternalToolExecutor` from `@roderai/sdk/browser` with your existing
notification loop. It deduplicates request ids within a binding, exposes an
`AbortSignal`, and suppresses late resolutions after cancellation. Browser
imports exclude Node process transports. `RoderAgent` automatically binds its
remote callbacks; custom hosted transports set `externalToolExecution: "hosted"`.
Local app-server calls remain in-process callbacks without a hosted connection.

Regenerate the method manifest and SDK inputs with:

```sh
mise exec -- cargo run -p roder-protocol --example export_app_server_schema
mise exec -- node sdk/codegen/generate-typescript.mjs
mise exec -- node sdk/codegen/generate-python.mjs
```

Dispatch callbacks without blocking the notification reader, so cancellation and
revocation can interrupt a callback that is awaiting an application operation:

```ts
const executor = await ExternalToolExecutor.bind(client, threadId, executeTool);
for await (const notification of client.notifications()) {
  void executor.handle(notification).catch(reportTransportError);
}
executor.stop();
```

Custom TypeScript transports must expose `closedSignal: AbortSignal` and abort it synchronously when the connection closes. The SDK executor observes that signal before any buffered requests can execute. Normal gateway disconnects await lease revocation; the drop guard also handles task cancellation.
