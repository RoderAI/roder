import assert from "node:assert/strict";
import test from "node:test";
import { InMemoryTransport, RoderAgent } from "../src/index.js";

test("agent resolves user input and plan exit callbacks", async () => {
  const methods: string[] = [];
  const requests: Array<{ method: string; params?: unknown }> = [];
  const transport = new InMemoryTransport((request) => {
    methods.push(request.method);
    requests.push({ method: request.method, params: request.params });
    return { jsonrpc: "2.0", id: request.id, result: { ok: true } };
  });
  await RoderAgent.create({
    transport,
    approvals: {
      onUserInput() {
        return { answers: "answer" };
      },
      onPlanExit() {
        return { approved: true };
      },
    },
  });

  transport.emit({
    jsonrpc: "2.0",
    method: "thread/userInputRequested",
    params: { requestId: "input-1" },
  });
  transport.emit({
    jsonrpc: "2.0",
    method: "thread/planExitRequested",
    params: { requestId: "plan-1" },
  });

  await eventually(() =>
    methods.includes("thread/resolve_user_input") && methods.includes("thread/exit_plan"),
  );
  assert.deepEqual(requests.filter((request) => request.method.startsWith("thread/")), [
    {
      method: "thread/resolve_user_input",
      params: { requestId: "input-1", answers: "answer" },
    },
    {
      method: "thread/exit_plan",
      params: { requestId: "plan-1", approved: true },
    },
  ]);
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
