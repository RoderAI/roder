import assert from "node:assert/strict";
import test from "node:test";
import { InMemoryTransport, RoderRpcClient, RoderRpcError, appServerMethods } from "../src/index.js";
import type { JsonRpcRequest } from "../src/index.js";

test("client calls typed manifest methods through helper map", async () => {
  const seen: JsonRpcRequest[] = [];
  const client = new RoderRpcClient(
    new InMemoryTransport((request) => {
      seen.push(request);
      return {
        jsonrpc: "2.0",
        id: request.id,
        result: { ok: true, method: request.method },
      };
    }),
  );

  const result = await client.methods["providers/list"]();

  assert.deepEqual(result, { ok: true, method: "providers/list" });
  assert.equal(seen[0]?.jsonrpc, "2.0");
  assert.equal(seen[0]?.method, "providers/list");
  assert.ok(appServerMethods.includes("thread/start"));
});

test("client preserves JSON-RPC errors with method and request id", async () => {
  const client = new RoderRpcClient(
    new InMemoryTransport((request) => ({
      jsonrpc: "2.0",
      id: request.id,
      error: { code: -32602, message: "bad params", data: { field: "threadId" } },
    })),
  );

  await assert.rejects(
    () => client.call("thread/read", { threadId: "" }),
    (error: unknown) => {
      assert.ok(error instanceof RoderRpcError);
      assert.equal(error.code, -32602);
      assert.equal(error.method, "thread/read");
      assert.deepEqual(error.data, { field: "threadId" });
      assert.equal(error.requestId, 1);
      return true;
    },
  );
});

test("client aborts pending requests", async () => {
  const controller = new AbortController();
  controller.abort();
  const client = new RoderRpcClient(
    new InMemoryTransport(() => ({
      jsonrpc: "2.0",
      id: 1,
      result: {},
    })),
  );

  await assert.rejects(() => client.call("providers/list", undefined, { signal: controller.signal }), {
    name: "AbortError",
  });
});
