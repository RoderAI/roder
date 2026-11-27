import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { AcpConnection } from "../src/transport.mjs";

const fixture = fileURLToPath(new URL("./fixtures/transport-agent.mjs", import.meta.url));

async function connection(t, config = {}, mode) {
  const client = new AcpConnection({
    command: process.execPath,
    args: [fixture, ...(mode ? [mode] : [])],
    ...config,
  });
  t.after(() => client.stop());
  const initialized = await client.start();
  assert.equal(initialized.protocolVersion, 1);
  assert.deepEqual(initialized.received, { protocolVersion: 1, clientCapabilities: {} });
  return client;
}

test("initializes once and correlates concurrent responses", async (t) => {
  const client = await connection(t);
  assert.equal(client.alive, true);
  assert.strictEqual(client.start(), client.start());
  const [slow, fast] = await Promise.all([
    client.request("echo", { value: "slow", delay: 30 }),
    client.request("echo", { value: "fast", delay: 1 }),
  ]);
  assert.equal(slow.value, "slow");
  assert.equal(fast.value, "fast");
  assert.equal(await client.request("fragmented"), "hello \u{1f680}");
});

test("routes notifications and asynchronous agent requests", async (t) => {
  let resolveNotification;
  const notified = new Promise((resolve) => { resolveNotification = resolve; });
  const client = await connection(t, {
    onNotification: (method, params) => resolveNotification({ method, params }),
    onRequest: async (method, params) => {
      assert.equal(method, "session/request_permission");
      await new Promise((resolve) => setTimeout(resolve, 5));
      return { outcome: params.allow ? "allowed" : "denied" };
    },
  });
  await client.notify("notification", { message: "updated" });
  assert.deepEqual(await notified, { method: "session/update", params: { message: "updated" } });
  const response = await client.request("callback", { allow: true });
  assert.equal(response.id, "permission");
  assert.deepEqual(response.result, { outcome: "allowed" });
});

test("serializes unsupported and rejected agent requests", async (t) => {
  const client = await connection(t);
  assert.equal((await client.request("callback", {})).error.code, -32601);
  client.onRequest = () => {
    const error = new Error("denied");
    error.rpcCode = -32002;
    throw error;
  };
  assert.deepEqual((await client.request("callback", {})).error, { code: -32002, message: "denied" });
});

test("preserves structured RPC errors without terminating the child", async (t) => {
  const client = await connection(t);
  await assert.rejects(client.request("error"), (error) => {
    assert.equal(error.rpcCode, -32001);
    assert.equal(error.data, "detail");
    return true;
  });
  assert.equal(client.alive, true);
});

test("timeouts and aborts only remove the affected request", async (t) => {
  const client = await connection(t);
  await assert.rejects(client.request("never", {}, { timeoutMs: 10 }), { name: "TimeoutError" });
  const controller = new AbortController();
  const pending = client.request("never", {}, { timeoutMs: 0, signal: controller.signal });
  controller.abort();
  await assert.rejects(pending, { name: "AbortError" });
  await assert.rejects(client.request("never", {}, { signal: controller.signal }), { name: "AbortError" });
  const result = await client.request("echo", { delay: 30 }, { timeoutMs: 0 });
  assert.equal(result.delay, 30);
  assert.equal(client.pending.size, 0);
  assert.equal(client.alive, true);
});

for (const method of ["malformed", "invalid-envelope", "invalid-utf8", "oversized", "exit"]) {
  test(`${method} rejects every pending request and terminates the connection`, async (t) => {
    const client = await connection(t, { maxMessageBytes: 1024 });
    const first = client.request("never");
    const second = client.request(method);
    const results = await Promise.allSettled([first, second]);
    assert.ok(results.every((result) => result.status === "rejected"));
    assert.equal(client.alive, false);
    await assert.rejects(client.request("echo"));
  });
}

test("inherits environment and bounds stderr", async (t) => {
  const previous = process.env.RODER_TRANSPORT_TEST;
  process.env.RODER_TRANSPORT_TEST = "gateway-secret-fixture";
  t.after(() => {
    if (previous === undefined) delete process.env.RODER_TRANSPORT_TEST;
    else process.env.RODER_TRANSPORT_TEST = previous;
  });
  const client = await connection(t);
  assert.equal(await client.request("stderr"), "gateway-secret-fixture");
  assert.ok(Buffer.byteLength(client.stderrTail) <= 64 * 1024);
  assert.ok(client.stderrTail.endsWith("x".repeat(100)));
});

test("stop rejects pending work and escalates for an uncooperative child", async (t) => {
  const client = await connection(t, {}, "stubborn");
  const pending = client.request("never");
  const rejected = assert.rejects(pending, /stopped/);
  const started = Date.now();
  await client.stop();
  await rejected;
  assert.ok(Date.now() - started < 2000);
  assert.equal(client.child.signalCode, "SIGKILL");
  assert.equal(client.alive, false);
  await client.stop();
});

test("spawn errors reject start without unhandled stream errors", async () => {
  const client = new AcpConnection({ command: "/nonexistent/roder-transport-test" });
  await assert.rejects(client.start(), /ENOENT/);
  await client.stop();
  assert.equal(client.alive, false);
});

test("rejects oversized outgoing messages without poisoning other work", async (t) => {
  const client = await connection(t, { maxMessageBytes: 1024 });
  await assert.rejects(client.request("echo", { value: "x".repeat(2048) }), /maxMessageBytes/);
  assert.deepEqual(await client.request("echo", { value: "ok" }), { value: "ok" });
});

test("ignores late responses to aborted requests and isolates callback failures", async (t) => {
  const client = await connection(t, {
    onNotification: async () => { throw new Error("consumer failure"); },
  });
  const controller = new AbortController();
  const pending = client.request("echo", { delay: 10 }, { signal: controller.signal });
  controller.abort();
  await assert.rejects(pending, { name: "AbortError" });
  await client.notify("notification", {});
  assert.deepEqual(await client.request("echo", { delay: 30 }), { delay: 30 });
  assert.equal(client.pending.size, 0);
  assert.equal(client.alive, true);
});

test("validates configuration and method inputs locally", async (t) => {
  assert.throws(() => new AcpConnection({ command: "" }), /command/);
  assert.throws(() => new AcpConnection({ command: "roder", requestTimeoutMs: -1 }), /requestTimeoutMs/);
  assert.throws(() => new AcpConnection({ command: "roder", maxMessageBytes: 0 }), /maxMessageBytes/);
  const client = await connection(t);
  await assert.rejects(client.request(""), /method/);
  await assert.rejects(client.notify(undefined), /method/);
  await assert.rejects(client.request("echo", {}, { timeoutMs: -1 }), /timeoutMs/);
  assert.equal(client.pending.size, 0);
});

test("stopping before start is terminal", async () => {
  const client = new AcpConnection({ command: process.execPath });
  await client.stop();
  await assert.rejects(client.start(), /stopped/);
  assert.equal(client.alive, false);
});

async function assertProcessGone(pid) {
  for (let attempt = 0; attempt < 200; attempt++) {
    try { process.kill(pid, 0); }
    catch (error) {
      if (error.code === "ESRCH") return;
      throw error;
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  assert.fail(`Process ${pid} remains after transport shutdown`);
}

test("EOF lets native cleanup finish before any termination signal", async (t) => {
  const client = await connection(t, {}, "graceful");
  await client.stop();
  assert.equal(client.child.exitCode, 0);
  assert.match(client.stderrTail, /graceful EOF complete/);
});

test("stop terminates and reaps an actual descendant in its owned POSIX group", {
  skip: process.platform === "win32" ? "Node only supports root-child termination on Windows" : false,
}, async (t) => {
  const client = await connection(t, {}, "descendant");
  const pid = await client.request("descendant-pid");
  assert.ok(Number.isInteger(pid) && pid > 0);
  const started = Date.now();
  await client.stop();
  assert.ok(Date.now() - started < 2000);
  await assertProcessGone(pid);
});

for (const method of ["exit", "malformed"]) {
  test(`${method} cleans descendants even when the native root exits first`, {
    skip: process.platform === "win32" ? "Node only supports root-child termination on Windows" : false,
  }, async (t) => {
    const client = await connection(t, {}, "descendant");
    const pid = await client.request("descendant-pid");
    await assert.rejects(client.request(method));
    await client.stop();
    await assertProcessGone(pid);
  });
}
