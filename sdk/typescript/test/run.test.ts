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

async function collect<T>(events: AsyncIterable<T>): Promise<T[]> {
  const items: T[] = [];
  for await (const event of events) {
    items.push(event);
  }
  return items;
}
