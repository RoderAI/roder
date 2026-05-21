import assert from "node:assert/strict";
import test from "node:test";
import { InMemoryTransport, RoderAgent } from "../src/index.js";

test("agent resolves user input and plan exit callbacks", async () => {
  const methods: string[] = [];
  const transport = new InMemoryTransport((request) => {
    methods.push(request.method);
    return { jsonrpc: "2.0", id: request.id, result: { ok: true } };
  });
  await RoderAgent.create({
    transport,
    approvals: {
      onUserInput() {
        return { response: "answer" };
      },
      onPlanExit() {
        return { accepted: true, message: "done" };
      },
    },
  });

  transport.emit({
    jsonrpc: "2.0",
    method: "session/userInputRequested",
    params: { requestId: "input-1" },
  });
  transport.emit({
    jsonrpc: "2.0",
    method: "session/planExitRequested",
    params: { requestId: "plan-1" },
  });

  await eventually(() =>
    methods.includes("session/resolve_user_input") && methods.includes("session/exit_plan"),
  );
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
