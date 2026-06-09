# Roder SDK

The Roder SDKs are thin clients for the Roder app-server JSON-RPC boundary. They do not wrap a model provider directly; they control Roder threads, turns, approvals, tools, commands, transports, and raw app-server notifications.

## Architecture

- `RoderRpcClient` is the low-level JSON-RPC client. It preserves request ids, app-server error codes, raw results, and notifications.
- `RoderAgent` is the high-level helper for starting or resuming a thread, sending input, streaming run events, handling approvals, and interrupting a run.
- `RoderRun` represents one active turn. It supports `stream`, `wait`, `cancel`, raw event access, and result reads.
- Transports are interchangeable: in-memory fake transport for tests, local stdio process transport for `roder app-server --listen stdio://`, and remote WebSocket transport for paired app-server instances.
- Generated type files come from `schemas/app-server/roder-app-server.v1.json`. Do not edit generated files by hand.

## Safety Defaults

High-level helpers do not automatically call destructive app-server methods. Read-only helpers cover model, provider, thread, tool, and command listing. Mutating methods such as command execution, plugin installs, memory writes, hunk rollback, and tool calls stay explicit low-level calls.

Live local or remote smoke checks are opt-in:

```sh
RODER_SDK_LIVE=1 node sdk/scripts/live-smoke.ts
RODER_SDK_LIVE=1 uv run --project sdk/python python sdk/scripts/live_smoke.py
```

Without `RODER_SDK_LIVE=1`, examples and tests use fake transports.

## TypeScript Quickstart

```ts
import { InMemoryTransport, RoderAgent } from "@roder/sdk";

const transport = new InMemoryTransport((request) => {
  if (request.method === "thread/start") {
    return { jsonrpc: "2.0", id: request.id, result: { thread: { id: "thread-1" } } };
  }
  if (request.method === "turn/start") {
    return { jsonrpc: "2.0", id: request.id, result: { turn: { id: "turn-1" } } };
  }
  return { jsonrpc: "2.0", id: request.id, result: {} };
});

const agent = await RoderAgent.create({ transport, cwd: "/workspace" });
const run = await agent.send("Summarize the repo");
transport.emit({ jsonrpc: "2.0", method: "turn/completed", params: { turnId: run.turnId } });
await run.wait();
```

## Python Quickstart

```py
from roder_sdk import InMemoryTransport, RoderAgent

async def handler(request):
    if request["method"] == "thread/start":
        return {"jsonrpc": "2.0", "id": request["id"], "result": {"thread": {"id": "thread-1"}}}
    if request["method"] == "turn/start":
        return {"jsonrpc": "2.0", "id": request["id"], "result": {"turn": {"id": "turn-1"}}}
    return {"jsonrpc": "2.0", "id": request["id"], "result": {}}

async with await RoderAgent.create(transport=InMemoryTransport(handler), cwd="/workspace") as agent:
    run = await agent.send("Summarize the repo")
```

## Error Handling

`RoderRpcError` preserves:

- `code`
- `message`
- `data`
- `method`
- `request id`

Transport failures raise transport-specific errors and do not rewrite app-server JSON-RPC errors.

## Events

Known notification methods are normalized to SDK event names:

| SDK event | App-server notification |
| --- | --- |
| `thread.started` | `thread/started` |
| `thread.status.changed` | `thread/status/changed` |
| `turn.started` | `turn/started` |
| `turn.completed` | `turn/completed` |
| `item.started` | `item/started` |
| `item.completed` | `item/completed` |
| `item.delta` | `item/agentMessage/delta`, `item/reasoning/textDelta`, `item/reasoning/summaryPartAdded`, `item/reasoning/summaryTextDelta` |
| `tool_execution.requested` | `thread/toolExecutionRequested` |
| `tool_execution.resolved` | `thread/toolExecutionResolved` |
| `approval.requested` | `thread/approvalRequested` |
| `approval.resolved` | `thread/approvalResolved` |
| `user_input.requested` | `thread/userInputRequested` |
| `user_input.resolved` | `thread/userInputResolved` |
| `plan_exit.requested` | `thread/planExitRequested` |
| `plan_exit.resolved` | `thread/planExitResolved` |
| `command.output_delta` | `command/exec/outputDelta` |

In the TypeScript SDK the thread, turn, item, delta, and tool-execution events carry typed payloads (`Thread`, `Turn`, `ThreadItem`, `ThreadItemDelta`, `ExternalToolCall`, `TokenUsage`) alongside `raw`; the interfaces in `sdk/typescript/src/events.ts` mirror the Rust wire structs in `crates/roder-protocol/src/lib.rs` and `crates/roder-api/src/inference.rs`. The Python SDK keeps the untyped `{type, raw}` shape.

Permissive mode yields unknown notifications as `raw.notification`; a known method whose payload fails parsing degrades the same way. Strict mode drops both.

## Method Groups

| Method group | Low-level helpers | High-level helpers |
| --- | --- | --- |
| providers/models | generated method map | `listProviders`, `listModels` / `list_providers`, `list_models` |
| thread | generated method map | `send`, `readThread`, `listThreads` / `send`, `read_thread`, `list_threads` |
| turn | generated method map | `RoderRun.cancel`, `RoderRun.wait` |
| thread approvals | generated method map | callback resolution |
| tools/commands | generated method map | `listTools`, `listCommands` / `list_tools`, `list_commands` |
| tasks/teams/plugins/media/memory/workflow | generated method map | explicit low-level calls |

## Compatibility

SDK compatibility follows `schemaVersion` in `roder-app-server.v1.json`. Raw clients may call unknown future methods because the manifest includes `unknownMethodsAllowed`; generated helper maps stay strict to known methods.

Remote WebSocket transports use bearer auth headers where supported. Deployments that cannot pass headers should use the documented remote pairing subprotocol flow rather than putting tokens in query strings.
