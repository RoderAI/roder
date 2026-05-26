import { InMemoryTransport, RoderAgent } from "../../typescript/src/index.js";

const transport = new InMemoryTransport((request) => ({
  jsonrpc: "2.0",
  id: request.id,
  result:
    request.method === "thread/start"
      ? { thread: { id: "thread-approval" } }
      : request.method === "turn/start"
        ? { turn: { id: "turn-approval" } }
        : { ok: true },
}));

const agent = await RoderAgent.create({
  transport,
  approvals: {
    onToolApproval(request) {
      const toolName = (request as { toolName?: string }).toolName;
      return { approved: toolName === "fs/readFile" };
    },
  },
});

await agent.send("Read a file only if policy allows it");
transport.emit({
  jsonrpc: "2.0",
  method: "thread/approvalRequested",
  params: { approvalId: "approval-example", toolName: "fs/readFile" },
});
await agent.close();
