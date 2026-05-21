import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";
import { InMemoryTransport, RoderAgent, RoderRpcClient } from "../src/index.js";
import type { JsonRpcRequest, JsonRpcResponse } from "../src/index.js";

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
  });
  const run = await agent.send("hello");
  const events = collectTypes(run.stream());
  emitNotifications(transport, fixture);

  assert.deepEqual(await events, ["turn.delta", "turn.completed"]);
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
        return { approved: true, message: "fixture approval" };
      },
    },
  });

  await agent.send("read file");
  emitNotifications(transport, fixture);

  await eventually(() => seen.includes("approval-1") && transport.seenMethods.includes("session/resolve_approval"));
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
        return { response: "fixture answer" };
      },
      onPlanExit() {
        return { accepted: true, message: "fixture plan accepted" };
      },
    },
  });

  await agent.send("ask me");
  emitNotifications(transport, fixture);

  await eventually(
    () =>
      transport.seenMethods.includes("session/resolve_user_input") &&
      transport.seenMethods.includes("session/exit_plan"),
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

async function eventually(assertion: () => boolean): Promise<void> {
  for (let i = 0; i < 20; i += 1) {
    if (assertion()) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  assert.ok(assertion());
}
