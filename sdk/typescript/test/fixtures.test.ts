import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";
import { InMemoryTransport, RoderAgent, RoderRpcClient } from "../src/index.js";
import type { JsonRpcRequest, JsonRpcResponse, RoderSdkEvent } from "../src/index.js";

const fixtureRoot = resolve(process.cwd(), "../fixtures/fake-app-server");

test("typescript sdk replays basic thread fixture", async () => {
  const fixture = loadFixture("basic-thread.jsonl");
  const transport = fixtureTransport(fixture);
  const client = new RoderRpcClient(transport);

  assert.equal(((await client.call("initialize", {})) as any).provider, "mock");
  assert.equal(((await client.methods["providers/list"]()) as any).providers[0].id, "mock");
  assert.equal(((await client.methods["model/list"]()) as any).models[0].id, "mock");

  const agent = await RoderAgent.create({
    transport,
    cwd: "/workspace",
    model: { provider: "mock", id: "mock" },
    toolAllowlist: ["edit", "read_file"],
    instructions: "You are embedded in a host app.",
  });
  const run = await agent.send("hello");
  const events = collectEvents(run.stream());
  emitNotifications(transport, fixture);

  const collected = await events;
  assert.deepEqual(
    collected.map((event) => event.type),
    ["item.started", "item.delta", "item.completed", "turn.completed"],
  );
  const delta = collected[1];
  assert.ok(delta?.type === "item.delta");
  assert.deepEqual(delta.delta, { type: "agentMessageText", delta: "hello" });
  const completed = collected.at(-1);
  assert.ok(completed?.type === "turn.completed");
  assert.equal(completed.turn.finishReason, "stop");
  assert.equal(completed.turn.usage?.cache_creation_prompt_tokens, 5);
  assert.equal(completed.turn.usage?.cached_prompt_tokens, 92);
});

test("typescript sdk replays approval fixture", async () => {
  const fixture = loadFixture("approval-flow.jsonl");
  const transport = fixtureTransport(fixture);
  const seen: string[] = [];
  const agent = await RoderAgent.create({
    transport,
    cwd: "/workspace",
    model: { provider: "mock", id: "mock" },
    approvals: {
      onToolApproval(request) {
        seen.push((request as { approvalId: string }).approvalId);
        return { approved: true };
      },
    },
  });

  await agent.send("read file");
  emitNotifications(transport, fixture);

  await eventually(() => seen.includes("approval-1") && transport.seenMethods.includes("thread/resolve_approval"));
});

test("typescript sdk replays external tool fixture", async () => {
  const fixture = loadFixture("external-tool-flow.jsonl");
  const transport = fixtureTransport(fixture);
  const calls: Array<{ id: string; name: string; arguments: unknown }> = [];
  const agent = await RoderAgent.create({
    transport,
    cwd: "/workspace",
    model: { provider: "mock", id: "mock" },
    externalTools: [
      {
        name: "acme_lookup",
        description: "Look up Acme workspace state.",
        parameters: {
          type: "object",
          properties: { query: { type: "string" } },
          required: ["query"],
        },
      },
    ],
    onToolExecute(call) {
      calls.push(call);
      return { output: "2 open threads" };
    },
  });

  const run = await agent.send("look up threads");
  /**
   * Consume the run stream concurrently with the agent's callback loop: both
   * must observe the full notification feed (callback loop resolves the tool,
   * the stream sees turn/completed).
   */
  const completion = run.wait();
  emitNotifications(transport, fixture);

  await eventually(() => transport.seenMethods.includes("tools/resolve"));
  assert.deepEqual(calls, [
    { id: "call-1", name: "acme_lookup", arguments: { query: "thread status" } },
  ]);
  const completed = await completion;
  assert.equal(completed?.turn.id, "turn-external");
});

test("typescript sdk replays runner thread fixture", async () => {
  const fixture = loadFixture("runner-thread-flow.jsonl");
  const transport = fixtureTransport(fixture);
  const agent = await RoderAgent.create({
    transport,
    cwd: "/local/scratch",
    workspaceId: "ws-fixture",
    model: { provider: "mock", id: "mock" },
    runner: {
      providerId: "e2b",
      config: { space_id: "space-1", mode: "readwrite" },
      workspace: "/workspace",
    },
  });

  const run = await agent.send("write a file");
  const completion = run.wait();
  emitNotifications(transport, fixture);

  const completed = await completion;
  assert.equal(completed?.turn.id, "turn-runner");
  // fixtureTransport already asserted the runner binding shape on thread/start.
  assert.ok(transport.seenMethods.includes("thread/start"));
});

test("typescript sdk replays user input and plan exit fixture", async () => {
  const fixture = loadFixture("user-input-flow.jsonl");
  const transport = fixtureTransport(fixture);
  const agent = await RoderAgent.create({
    transport,
    cwd: "/workspace",
    model: { provider: "mock", id: "mock" },
    approvals: {
      onUserInput() {
        return { answers: "fixture answer" };
      },
      onPlanExit() {
        return { approved: true };
      },
    },
  });

  await agent.send("ask me");
  emitNotifications(transport, fixture);

  await eventually(
    () =>
      transport.seenMethods.includes("thread/resolve_user_input") &&
      transport.seenMethods.includes("thread/exit_plan"),
  );
});

test("typescript sdk replays command output and interrupt fixture", async () => {
  const fixture = loadFixture("command-output-flow.jsonl");
  const transport = fixtureTransport(fixture);
  const agent = await RoderAgent.create({
    transport,
    cwd: "/workspace",
    model: { provider: "mock", id: "mock" },
  });
  const run = await agent.send("run command");
  const events = collectTypes(run.stream());
  emitNotifications(transport, fixture);

  assert.deepEqual(await run.cancel("fixture stop"), { interrupted: true });
  assert.deepEqual(await events, ["command.output_delta", "turn.completed"]);
});

test("typescript sdk replays workspace files fixture", async () => {
  const fixture = loadFixture("workspace-files-flow.jsonl");
  const transport = fixtureTransport(fixture);
  const client = new RoderRpcClient(transport);

  const status = (await client.call("workspace/files/status", { workspaceId: "ws_files" })) as any;
  assert.equal(status.status.state, "missing");

  const rebuild = (await client.call("workspace/files/rebuild", { workspaceId: "ws_files" })) as any;
  assert.equal(rebuild.status.state, "ready");
  assert.equal(rebuild.status.fileCount, 3);

  const rootChildren = (await client.call("workspace/files/children", {
    workspaceId: "ws_files",
    rootId: "root_repo",
  })) as any;
  assert.deepEqual(
    rootChildren.entries.map((entry: any) => entry.name),
    ["roadmap", "src"],
  );

  const roadmapChildren = (await client.call("workspace/files/children", {
    workspaceId: "ws_files",
    rootId: "root_repo",
    path: "roadmap",
  })) as any;
  assert.equal(roadmapChildren.entries[0].kind, "file");

  const query = (await client.call("workspace/files/query", {
    workspaceId: "ws_files",
    query: "desktop custom",
    limit: 5,
  })) as any;
  assert.equal(query.matches[0].entry.path, "roadmap/001-desktop-custom-user-extensions.md");

  const directoryQuery = (await client.call("workspace/files/query", {
    workspaceId: "ws_files",
    query: "roadmap",
    limit: 5,
  })) as any;
  assert.equal(directoryQuery.matches[0].entry.path, "roadmap");
  assert.equal(directoryQuery.matches[0].entry.kind, "directory");

  const read = (await client.call("workspace/files/read", {
    workspaceId: "ws_files",
    rootId: "root_repo",
    path: "roadmap/001-desktop-custom-user-extensions.md",
    limit: 17,
  })) as any;
  assert.equal(read.encoding, "utf8");
  assert.equal(read.text, "# Desktop Custom ");

  assert.deepEqual(
    fixture.notifications.map((notification) => notification.method),
    ["workspace/files/statusChanged", "workspace/files/statusChanged"],
  );
});

type Fixture = {
  requests: JsonRpcRequest[];
  responses: JsonRpcResponse[];
  notifications: Array<{ jsonrpc: "2.0"; method: string; params?: unknown }>;
};

function loadFixture(name: string): Fixture {
  const records = readFileSync(resolve(fixtureRoot, name), "utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  return {
    requests: records.filter((record) => record.kind === "api.request").map((record) => record.request),
    responses: records.filter((record) => record.kind === "api.response").map((record) => record.response),
    notifications: records
      .filter((record) => record.kind === "api.notification")
      .map((record) => record.notification),
  };
}

function fixtureTransport(fixture: Fixture): InMemoryTransport & { seenMethods: string[] } {
  const requests = [...fixture.requests];
  const responses = [...fixture.responses];
  const seenMethods: string[] = [];
  const transport = new InMemoryTransport((request) => {
    const expected = requests.shift();
    const response = responses.shift();
    assert.equal(request.method, expected?.method);
    assert.deepEqual(request.params ?? {}, expected?.params ?? {});
    seenMethods.push(request.method);
    return { ...(response ?? { jsonrpc: "2.0", id: request.id, result: {} }), id: request.id };
  }) as InMemoryTransport & { seenMethods: string[] };
  transport.seenMethods = seenMethods;
  return transport;
}

function emitNotifications(transport: InMemoryTransport, fixture: Fixture): void {
  for (const notification of fixture.notifications) {
    transport.emit(notification);
  }
}

async function collectTypes(events: AsyncIterable<{ type: string }>): Promise<string[]> {
  const types: string[] = [];
  for await (const event of events) {
    types.push(event.type);
  }
  return types;
}

async function collectEvents(events: AsyncIterable<RoderSdkEvent>): Promise<RoderSdkEvent[]> {
  const collected: RoderSdkEvent[] = [];
  for await (const event of events) {
    collected.push(event);
  }
  return collected;
}

async function eventually(assertion: () => boolean): Promise<void> {
  for (let i = 0; i < 20; i += 1) {
    if (assertion()) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  assert.ok(assertion());
}
