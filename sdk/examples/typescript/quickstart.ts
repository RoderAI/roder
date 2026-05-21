import { InMemoryTransport, RoderAgent } from "../../typescript/src/index.js";

const transport = new InMemoryTransport((request) => {
  if (request.method === "thread/start") {
    return { jsonrpc: "2.0", id: request.id, result: { thread: { id: "thread-example" } } };
  }
  if (request.method === "turn/start") {
    return { jsonrpc: "2.0", id: request.id, result: { turn: { id: "turn-example" } } };
  }
  return { jsonrpc: "2.0", id: request.id, result: {} };
});

const agent = await RoderAgent.create({ transport, cwd: "/workspace" });
const run = await agent.send("Summarize this repository");
transport.emit({ jsonrpc: "2.0", method: "turn/completed", params: { turnId: run.turnId } });
await run.wait();
await agent.close();
