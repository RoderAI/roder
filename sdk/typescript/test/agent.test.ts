import assert from "node:assert/strict";
import test from "node:test";
import { InMemoryTransport, RoderAgent } from "../src/index.js";
import type { JsonRpcRequest } from "../src/index.js";

test("agent send starts a thread and turn", async () => {
  const requests: JsonRpcRequest[] = [];
  const transport = new InMemoryTransport((request) => {
    requests.push(request);
    if (request.method === "workspace/list") {
      return { jsonrpc: "2.0", id: request.id, result: { workspaces: [] } };
    }
    if (request.method === "workspace/create") {
      return { jsonrpc: "2.0", id: request.id, result: { workspace: { id: "ws-1" } } };
    }
    if (request.method === "thread/start") {
      return { jsonrpc: "2.0", id: request.id, result: { thread: { id: "thread-1" } } };
    }
    if (request.method === "turn/start") {
      return { jsonrpc: "2.0", id: request.id, result: { turn: { id: "turn-1" } } };
    }
    return { jsonrpc: "2.0", id: request.id, result: {} };
  });
  const agent = await RoderAgent.create({
    transport,
    cwd: "/workspace",
    model: { provider: "openai", id: "gpt-5.5" },
  });

  const run = await agent.send("hello");

  assert.equal(run.threadId, "thread-1");
  assert.equal(run.turnId, "turn-1");
  assert.equal(requests[0]?.method, "workspace/list");
  assert.equal(requests[1]?.method, "workspace/create");
  assert.deepEqual(requests[1]?.params, {
    roots: [{ path: "/workspace" }],
  });
  assert.equal(requests[2]?.method, "thread/start");
  assert.deepEqual(requests[2]?.params, {
    cwd: "/workspace",
    model: "gpt-5.5",
    modelProvider: "openai",
    workspaceId: "ws-1",
  });
  assert.equal(requests[3]?.method, "turn/start");
  assert.deepEqual(requests[3]?.params, {
    threadId: "thread-1",
    input: [{ type: "text", text: "hello" }],
  });
});

test("agent read-only helpers call safe app-server methods", async () => {
  const methods: string[] = [];
  const agent = await RoderAgent.create({
    threadId: "thread-1",
    transport: new InMemoryTransport((request) => {
      methods.push(request.method);
      return { jsonrpc: "2.0", id: request.id, result: { ok: true } };
    }),
  });

  await agent.listModels();
  await agent.listProviders();
  await agent.readThread();
  await agent.listThreads();
  await agent.listTools();
  await agent.listCommands();

  assert.deepEqual(methods, [
    "model/list",
    "providers/list",
    "thread/read",
    "thread/list",
    "tools/list",
    "commands/list",
  ]);
});

test("agent approval callback resolves approval requests", async () => {
  const methods: string[] = [];
  const transport = new InMemoryTransport((request) => {
    methods.push(request.method);
    return { jsonrpc: "2.0", id: request.id, result: { resolved: true } };
  });
  await RoderAgent.create({
    transport,
    approvals: {
      onToolApproval(request) {
        assert.deepEqual(request, { approvalId: "approval-1", toolName: "fs/readFile" });
        return { approved: true };
      },
    },
  });

  transport.emit({
    jsonrpc: "2.0",
    method: "thread/approvalRequested",
    params: { approvalId: "approval-1", toolName: "fs/readFile" },
  });
  await eventually(() => methods.includes("thread/resolve_approval"));
});

async function eventually(assertion: () => boolean): Promise<void> {
  for (let i = 0; i < 20; i += 1) {
    if (assertion()) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  assert.ok(assertion());
}
