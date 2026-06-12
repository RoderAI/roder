import assert from "node:assert/strict";
import test from "node:test";
import { InMemoryTransport, RoderRpcClient, RoderRun } from "../src/index.js";

test("run streams normalized events until its turn completes", async () => {
  const transport = new InMemoryTransport((request) => ({
    jsonrpc: "2.0",
    id: request.id,
    result: {},
  }));
  const run = new RoderRun(new RoderRpcClient(transport), "thread-1", "turn-1");
  const events = collect(run.stream());

  transport.emit({
    jsonrpc: "2.0",
    method: "item/agentMessage/delta",
    params: {
      seq: 1,
      eventId: "turn-1-item-event-1",
      threadId: "thread-1",
      turnId: "turn-1",
      timestamp: "1970-01-01T00:00:00Z",
      event: {
        type: "itemDelta",
        itemId: "turn-1-agent-final_answer",
        delta: { type: "agentMessageText", delta: "hello" },
      },
    },
  });
  transport.emit({
    jsonrpc: "2.0",
    method: "turn/completed",
    params: {
      threadId: "thread-1",
      turn: { id: "turn-1", items: [], itemsView: "default", status: "completed" },
    },
  });

  assert.deepEqual(
    (await events).map((event) => event.type),
    ["item.delta", "turn.completed"],
  );
});

test("run wait ignores other threads' and turns' completions", async () => {
  const transport = new InMemoryTransport((request) => ({
    jsonrpc: "2.0",
    id: request.id,
    result: {},
  }));
  const run = new RoderRun(new RoderRpcClient(transport), "thread-1", "turn-2");
  const waiting = run.wait();

  for (const [threadId, turnId] of [
    ["thread-9", "turn-9"],
    ["thread-1", "turn-1"],
    ["thread-1", "turn-2"],
  ]) {
    transport.emit({
      jsonrpc: "2.0",
      method: "turn/completed",
      params: {
        threadId,
        turn: { id: turnId, items: [], itemsView: "default", status: "completed" },
      },
    });
  }

  const completed = await waiting;
  assert.equal(completed?.threadId, "thread-1");
  assert.equal(completed?.turn.id, "turn-2");
});

test("run cancel maps to turn interrupt", async () => {
  let interruptParams: unknown;
  const run = new RoderRun(
    new RoderRpcClient(
      new InMemoryTransport((request) => {
        if (request.method === "turn/interrupt") {
          interruptParams = request.params;
        }
        return { jsonrpc: "2.0", id: request.id, result: { interrupted: true } };
      }),
    ),
    "thread-1",
    "turn-1",
  );

  assert.deepEqual(await run.cancel("stop"), { interrupted: true });
  assert.deepEqual(interruptParams, {
    threadId: "thread-1",
    turnId: "turn-1",
    reason: "stop",
  });
});

test("run stream aborts a parked read when its signal fires", async () => {
  const transport = new InMemoryTransport((request) => ({
    jsonrpc: "2.0",
    id: request.id,
    result: {},
  }));
  const run = new RoderRun(new RoderRpcClient(transport), "thread-1", "turn-1");
  const controller = new AbortController();
  const consumed = collect(run.stream({ signal: controller.signal }));

  // No notifications are emitted, so the read parks; abort must unwedge it.
  await new Promise((resolve) => setTimeout(resolve, 10));
  controller.abort();

  await assert.rejects(consumed, (error: Error) => error.name === "AbortError");
});

test("run stream with an already-aborted signal rejects immediately", async () => {
  const transport = new InMemoryTransport((request) => ({
    jsonrpc: "2.0",
    id: request.id,
    result: {},
  }));
  const run = new RoderRun(new RoderRpcClient(transport), "thread-1", "turn-1");

  await assert.rejects(
    collect(run.stream({ signal: AbortSignal.abort() })),
    (error: Error) => error.name === "AbortError",
  );
});

test("run stream still completes normally when a signal is provided but never fires", async () => {
  const transport = new InMemoryTransport((request) => ({
    jsonrpc: "2.0",
    id: request.id,
    result: {},
  }));
  const run = new RoderRun(new RoderRpcClient(transport), "thread-1", "turn-1");
  const controller = new AbortController();
  const events = collect(run.stream({ signal: controller.signal }));

  transport.emit({
    jsonrpc: "2.0",
    method: "turn/completed",
    params: {
      threadId: "thread-1",
      turn: { id: "turn-1", items: [], itemsView: "default", status: "completed" },
    },
  });

  assert.deepEqual(
    (await events).map((event) => event.type),
    ["turn.completed"],
  );
});

async function collect<T>(events: AsyncIterable<T>): Promise<T[]> {
  const items: T[] = [];
  for await (const event of events) {
    items.push(event);
  }
  return items;
}
