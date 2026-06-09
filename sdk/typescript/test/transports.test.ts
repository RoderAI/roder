import assert from "node:assert/strict";
import test from "node:test";
import {
  InMemoryTransport,
  LocalProcessTransport,
  WebSocketTransport,
  type WebSocketLike,
} from "../src/index.js";

test("in-memory transport preserves notification order", async () => {
  const transport = new InMemoryTransport((request) => ({
    jsonrpc: "2.0",
    id: request.id,
    result: { ok: true },
  }));
  const iterator = transport.notifications()[Symbol.asyncIterator]();

  transport.emit({ jsonrpc: "2.0", method: "first", params: { n: 1 } });
  transport.emit({ jsonrpc: "2.0", method: "second", params: { n: 2 } });

  assert.equal((await iterator.next()).value.method, "first");
  assert.equal((await iterator.next()).value.method, "second");
  transport.close();
});

test("notifications fan out to every concurrent subscriber", async () => {
  const transport = new InMemoryTransport((request) => ({
    jsonrpc: "2.0",
    id: request.id,
    result: { ok: true },
  }));
  const first = transport.notifications()[Symbol.asyncIterator]();
  const second = transport.notifications()[Symbol.asyncIterator]();

  transport.emit({ jsonrpc: "2.0", method: "shared", params: { n: 1 } });

  assert.equal((await first.next()).value.method, "shared");
  assert.equal((await second.next()).value.method, "shared");
  transport.close();
});

test("notifications emitted before any subscriber replay to the first subscriber", async () => {
  const transport = new InMemoryTransport((request) => ({
    jsonrpc: "2.0",
    id: request.id,
    result: { ok: true },
  }));
  transport.emit({ jsonrpc: "2.0", method: "early", params: { n: 1 } });

  const iterator = transport.notifications()[Symbol.asyncIterator]();
  assert.equal((await iterator.next()).value.method, "early");
  transport.close();
});

test("local process transport exchanges json lines without a roder binary", async () => {
  const script = `
    const readline = require("node:readline");
    const rl = readline.createInterface({ input: process.stdin });
    console.log(JSON.stringify({ jsonrpc: "2.0", method: "process/ready", params: {} }));
    rl.on("line", line => {
      const request = JSON.parse(line);
      console.log(JSON.stringify({ jsonrpc: "2.0", id: request.id, result: { method: request.method, params: request.params } }));
    });
  `;
  const transport = new LocalProcessTransport({
    command: process.execPath,
    args: ["-e", script],
  });
  const notifications = transport.notifications()[Symbol.asyncIterator]();

  assert.equal((await notifications.next()).value.method, "process/ready");
  const response = await transport.request({
    jsonrpc: "2.0",
    id: "req-1",
    method: "commands/list",
    params: { limit: 1 },
  });

  assert.deepEqual(response.result, { method: "commands/list", params: { limit: 1 } });
  await transport.close();
});

test("local process transport drains chatty stderr and reports a bounded tail on exit", async () => {
  /**
   * The child floods stderr (~200KB, well past the 64KB pipe buffer) before
   * reading stdin. Without a continuous stderr drain it would block on the
   * full pipe and never answer the request.
   */
  const script = `
    for (let i = 0; i < 2000; i++) {
      process.stderr.write("stderr noise line " + i + " " + "x".repeat(80) + "\\n");
    }
    const readline = require("node:readline");
    const rl = readline.createInterface({ input: process.stdin });
    rl.on("line", line => {
      const request = JSON.parse(line);
      if (request.method === "process/crash") {
        process.stderr.write("fatal: boom before exit\\n");
        process.exit(3);
        return;
      }
      console.log(JSON.stringify({ jsonrpc: "2.0", id: request.id, result: { ok: true } }));
    });
  `;
  const transport = new LocalProcessTransport({
    command: process.execPath,
    args: ["-e", script],
  });

  const response = await transport.request({
    jsonrpc: "2.0",
    id: "req-1",
    method: "providers/list",
  });
  assert.deepEqual(response.result, { ok: true });

  const failed = transport.request({
    jsonrpc: "2.0",
    id: "req-2",
    method: "process/crash" as never,
  });
  const error = await failed.then(
    () => assert.fail("expected the crashed request to reject"),
    (caught: unknown) => caught as Error,
  );

  assert.equal(error.name, "RoderTransportError");
  assert.match(error.message, /app-server exited code=3 signal=null/);
  assert.match(error.message, /recent stderr \(last 50 lines\):/);
  assert.match(error.message, /fatal: boom before exit/);
  assert.match(error.message, /stderr noise line 1999 /);
  assert.doesNotMatch(error.message, /stderr noise line 1000 /);
  const tailNoiseLines = error.message.match(/stderr noise line /g) ?? [];
  assert.equal(tailNoiseLines.length, 49);
  await transport.close();
});

test("websocket transport sends bearer headers and resolves responses", async () => {
  let socket!: FakeWebSocket;
  const transport = new WebSocketTransport({
    url: "ws://127.0.0.1:1234",
    token: "secret-token",
    webSocketFactory(url, protocols, options) {
      socket = new FakeWebSocket(url, protocols, options);
      return socket;
    },
  });

  assert.equal(socket.url, "ws://127.0.0.1:1234");
  assert.deepEqual(socket.options.headers, { Authorization: "Bearer secret-token" });
  socket.open();
  const pending = transport.request({
    jsonrpc: "2.0",
    id: 7,
    method: "providers/list",
  });
  await Promise.resolve();
  assert.equal(JSON.parse(socket.sent[0] ?? "{}").method, "providers/list");
  socket.message({ jsonrpc: "2.0", id: 7, result: { providers: [] } });

  assert.deepEqual((await pending).result, { providers: [] });
  transport.close();
});

test("websocket transport streams notifications", async () => {
  let socket!: FakeWebSocket;
  const transport = new WebSocketTransport({
    url: "ws://127.0.0.1:1234",
    webSocketFactory(url, protocols, options) {
      socket = new FakeWebSocket(url, protocols, options);
      return socket;
    },
  });
  socket.open();
  const iterator = transport.notifications()[Symbol.asyncIterator]();
  socket.message({ jsonrpc: "2.0", method: "thread/status/changed", params: { status: "idle" } });

  assert.equal((await iterator.next()).value.method, "thread/status/changed");
  transport.close();
});

class FakeWebSocket extends EventTarget implements WebSocketLike {
  readyState = 0;
  readonly sent: string[] = [];

  constructor(
    readonly url: string,
    readonly protocols: string[],
    readonly options: { headers?: Record<string, string> },
  ) {
    super();
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.readyState = 3;
    this.dispatchEvent(new Event("close"));
  }

  open(): void {
    this.readyState = 1;
    this.dispatchEvent(new Event("open"));
  }

  message(message: unknown): void {
    this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(message) }));
  }
}
