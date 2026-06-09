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

test("agent send passes tool allowlist and instructions on thread/start", async () => {
  const requests: JsonRpcRequest[] = [];
  const transport = new InMemoryTransport((request) => {
    requests.push(request);
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
    workspaceId: "ws-1",
    toolAllowlist: ["edit", "read_file"],
    instructions: "You are embedded in Sauna.",
  });

  await agent.send("hello");

  const threadStart = requests.find((request) => request.method === "thread/start");
  assert.deepEqual(threadStart?.params, {
    cwd: "/workspace",
    model: undefined,
    modelProvider: undefined,
    toolAllowlist: ["edit", "read_file"],
    developerInstructions: "You are embedded in Sauna.",
    workspaceId: "ws-1",
  });
});

test("agent registers external tools and resolves calls, including thrown errors", async () => {
  const requests: JsonRpcRequest[] = [];
  const transport = new InMemoryTransport((request) => {
    requests.push(request);
    if (request.method === "thread/start") {
      return { jsonrpc: "2.0", id: request.id, result: { thread: { id: "thread-1" } } };
    }
    if (request.method === "turn/start") {
      return { jsonrpc: "2.0", id: request.id, result: { turn: { id: "turn-1" } } };
    }
    return { jsonrpc: "2.0", id: request.id, result: { resolved: true } };
  });
  const externalTools = [
    { name: "sauna_lookup", description: "Look up Sauna state.", parameters: { type: "object" } },
  ];
  const agent = await RoderAgent.create({
    transport,
    cwd: "/workspace",
    workspaceId: "ws-1",
    externalTools,
    onToolExecute(call) {
      if (call.name === "sauna_lookup") {
        return { output: "2 open threads" };
      }
      throw new Error("unknown external tool");
    },
  });

  await agent.send("hello");

  const threadStart = requests.find((request) => request.method === "thread/start");
  assert.deepEqual(threadStart?.params, {
    cwd: "/workspace",
    model: undefined,
    modelProvider: undefined,
    externalTools,
    workspaceId: "ws-1",
  });

  transport.emit({
    jsonrpc: "2.0",
    method: "thread/toolExecutionRequested",
    params: {
      threadId: "thread-1",
      turnId: "turn-1",
      requestId: "exttool-1",
      call: { id: "call-1", name: "sauna_lookup", arguments: { query: "threads" } },
    },
  });
  transport.emit({
    jsonrpc: "2.0",
    method: "thread/toolExecutionRequested",
    params: {
      threadId: "thread-1",
      turnId: "turn-1",
      requestId: "exttool-2",
      call: { id: "call-2", name: "other_tool", arguments: {} },
    },
  });

  await eventually(
    () => requests.filter((request) => request.method === "tools/resolve").length === 2,
  );
  assert.deepEqual(
    requests
      .filter((request) => request.method === "tools/resolve")
      .map((request) => request.params),
    [
      { requestId: "exttool-1", output: "2 open threads", isError: false },
      { requestId: "exttool-2", output: "Error: unknown external tool", isError: true },
    ],
  );
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
