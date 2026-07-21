// Hosted SDK helpers (phase 72, Task 6): bearer auth at handshake, typed
// hosted/* helpers, token-provider reconnect without request replay, and
// raw JSON-RPC access. Fully offline against a fake WebSocket.

import assert from "node:assert/strict";
import test from "node:test";
import { HostedClient } from "../src/index.js";
import type { WebSocketLike } from "../src/index.js";

class FakeHostedSocket extends EventTarget implements WebSocketLike {
  readyState = 0;
  readonly sent: string[] = [];

  constructor(
    readonly url: string,
    readonly protocols: string[],
    readonly options: { headers?: Record<string, string> },
  ) {
    super();
    queueMicrotask(() => {
      this.readyState = 1;
      this.dispatchEvent(new Event("open"));
    });
  }

  send(data: string): void {
    this.sent.push(data);
    const request = JSON.parse(data);
    const respond = (result: unknown) =>
      queueMicrotask(() =>
        this.dispatchEvent(
          new MessageEvent("message", {
            data: JSON.stringify({ jsonrpc: "2.0", id: request.id, result }),
          }),
        ),
      );
    switch (request.method) {
      case "hosted/whoami":
        respond({
          tenant: { tenantId: "acme" },
          principal: { kind: "user", user_id: "ops" },
          role: "tenant_admin",
          scopes: ["read", "write", "admin"],
        });
        break;
      case "hosted/service_accounts/create":
        respond({ keyId: "k1", token: "rk_sa_k1.secret" });
        break;
      case "hosted/service_accounts/revoke":
        respond({ revoked: true });
        break;
      case "hosted/hooks/list":
        respond({ hooks: [] });
        break;
      default:
        respond({ echoed: request.method });
    }
  }

  close(): void {
    this.readyState = 3;
    this.dispatchEvent(new Event("close"));
  }
}

function factoryCapture() {
  const sockets: FakeHostedSocket[] = [];
  const webSocketFactory = (
    url: string,
    protocols: string[],
    options: { headers?: Record<string, string> },
  ) => {
    const socket = new FakeHostedSocket(url, protocols, options);
    sockets.push(socket);
    return socket;
  };
  return { sockets, webSocketFactory };
}

test("hosted client authenticates with a bearer header and serves typed helpers", async () => {
  const { sockets, webSocketFactory } = factoryCapture();
  const hosted = await HostedClient.connect({
    url: "wss://roder.example.test",
    token: "rk_test_sdk_token",
    bearerAuth: "header",
    webSocketFactory,
  });

  assert.equal(sockets[0]!.options.headers?.Authorization, "Bearer rk_test_sdk_token");
  // Credentials never appear in the URL.
  assert.ok(!sockets[0]!.url.includes("rk_test"));

  const whoami = await hosted.whoami();
  assert.equal(whoami.tenant.tenantId, "acme");
  assert.equal(whoami.role, "tenant_admin");

  const minted = await hosted.createServiceAccount("ci");
  assert.ok(minted.token.startsWith("rk_sa_"));
  assert.equal((await hosted.revokeServiceAccount(minted.keyId)).revoked, true);
  assert.deepEqual((await hosted.listHooks()).hooks, []);

  // Raw JSON-RPC stays available for forward-compatible hosted methods.
  const raw = await hosted.client.call("hosted/tenants/list", {});
  assert.deepEqual(raw, { echoed: "hosted/tenants/list" });

  await hosted.close();
});

test("hosted client uses browser-safe bearer subprotocols by default", async () => {
  const previousWebSocket = Object.getOwnPropertyDescriptor(globalThis, "WebSocket");
  let socket: FakeHostedSocket | undefined;
  let constructorArgumentCount = 0;

  class BrowserHostedSocket extends FakeHostedSocket {
    constructor(...args: [url: string, protocols: string[]]) {
      constructorArgumentCount = args.length;
      super(args[0], args[1], {});
      socket = this;
    }
  }

  Object.defineProperty(globalThis, "WebSocket", {
    configurable: true,
    writable: true,
    value: BrowserHostedSocket,
  });

  try {
    const hosted = await HostedClient.connect({
      url: "wss://roder.example.test",
      token: "rk_test_browser_token",
    });
    await hosted.whoami();

    assert.equal(constructorArgumentCount, 2);
    assert.deepEqual(socket?.protocols, [
      "roder.remote.v1",
      "bearer.rk_test_browser_token",
    ]);
    assert.ok(!socket?.url.includes("rk_test_browser_token"));

    await hosted.close();
  } finally {
    if (previousWebSocket) {
      Object.defineProperty(globalThis, "WebSocket", previousWebSocket);
    } else {
      Reflect.deleteProperty(globalThis, "WebSocket");
    }
  }
});

test("hosted client reconnects with a fresh token and never replays requests", async () => {
  const { sockets, webSocketFactory } = factoryCapture();
  const issued: string[] = [];
  let serial = 0;
  const hosted = await HostedClient.connect({
    url: "wss://roder.example.test",
    tokenProvider: () => {
      serial += 1;
      const token = `rk_test_rotating_${serial}`;
      issued.push(token);
      return token;
    },
    bearerAuth: "header",
    webSocketFactory,
  });
  await hosted.whoami();
  const firstSent = sockets[0]!.sent.length;

  await hosted.reconnect();
  assert.equal(sockets.length, 2);
  assert.deepEqual(issued, ["rk_test_rotating_1", "rk_test_rotating_2"]);
  assert.equal(sockets[1]!.options.headers?.Authorization, "Bearer rk_test_rotating_2");
  // Nothing from the old connection is replayed on the new one.
  assert.equal(sockets[0]!.sent.length, firstSent);
  assert.equal(sockets[1]!.sent.length, 0);

  await hosted.whoami();
  assert.equal(sockets[1]!.sent.length, 1);
  await hosted.close();
});

test("hosted client requires a credential source and accepts external headers", async () => {
  const { webSocketFactory } = factoryCapture();
  await assert.rejects(
    HostedClient.connect({ url: "wss://roder.example.test", webSocketFactory }),
    /token, tokenProvider, or an Authorization header/,
  );

  const { sockets, webSocketFactory: factory2 } = factoryCapture();
  const hosted = await HostedClient.connect({
    url: "wss://roder.example.test",
    headers: { Authorization: "Bearer external-token" },
    webSocketFactory: factory2,
  });
  assert.equal(sockets[0]!.options.headers?.Authorization, "Bearer external-token");
  await hosted.close();
});
