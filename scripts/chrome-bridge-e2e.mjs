#!/usr/bin/env node
// Faithful end-to-end check of the Roder Chrome browser bridge WITHOUT a real
// browser. Opens two WebSocket connections to a running `roder app-server
// --remote`:
//
//   A) the "extension": authenticates with the bearer subprotocol, sends the
//      `hello` frame, and answers `{type,id,...}` command frames with
//      `{type:"command/result",id,ok,result}` — exactly like the MV3 extension.
//   B) a JSON-RPC "client": calls chrome/* methods. Because both connections hit
//      the same app-server process, chrome/* handlers see the registered
//      extension and dispatch commands to it.
//
// Usage: node chrome-bridge-e2e.mjs <wsUrl> <token>
// Exit 0 = PASS, 1 = FAIL. Uses Node's global WebSocket (Node >= 22).

const [, , url, token] = process.argv;
if (!url || !token) {
  console.error("usage: chrome-bridge-e2e.mjs <wsUrl> <token>");
  process.exit(2);
}

const SUBPROTOCOLS = ["roder.remote.v1", `bearer.${token}`];
const FAKE_TABS = [
  { id: 1, windowId: 1, title: "Example Domain", url: "https://example.com/", active: true },
  { id: 2, windowId: 1, title: "Roder", url: "https://roder.dev/", active: false },
];

function open(label) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url, SUBPROTOCOLS);
    ws.addEventListener("open", () => resolve(ws));
    ws.addEventListener("error", (e) =>
      reject(new Error(`${label} ws error: ${e.message ?? e.type ?? "unknown"}`)),
    );
  });
}

// JSON-RPC request/response over a client connection.
function rpc(ws, method, params) {
  return new Promise((resolve, reject) => {
    const id = `rpc-${Math.random().toString(36).slice(2)}`;
    const onMessage = (ev) => {
      let msg;
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }
      if (msg.id !== id) return;
      ws.removeEventListener("message", onMessage);
      if (msg.error) reject(new Error(`${method}: ${msg.error.message}`));
      else resolve(msg.result);
    };
    ws.addEventListener("message", onMessage);
    ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    setTimeout(() => {
      ws.removeEventListener("message", onMessage);
      reject(new Error(`${method}: timed out waiting for response`));
    }, 8000);
  });
}

function fail(message) {
  console.error(`FAIL: ${message}`);
  process.exit(1);
}

async function main() {
  // A) the extension.
  const ext = await open("extension");
  ext.addEventListener("message", (ev) => {
    let cmd;
    try {
      cmd = JSON.parse(ev.data);
    } catch {
      return;
    }
    // Command frames are `{type, id, ...}` and are not JSON-RPC.
    if (!cmd || typeof cmd.type !== "string" || cmd.id == null || cmd.jsonrpc) return;
    let result = { ok: true };
    if (cmd.type === "tabs/list") result = { tabs: FAKE_TABS };
    else if (cmd.type === "page/snapshot")
      result = { title: "Example Domain", url: "https://example.com/", text: "Example Domain", controls: [] };
    console.log(`  extension <- command ${cmd.type} (${cmd.id})`);
    ext.send(JSON.stringify({ type: "command/result", id: cmd.id, ok: true, result }));
  });
  ext.send(
    JSON.stringify({
      type: "hello",
      source: "roder-chrome",
      version: "0.2.0",
      capabilities: ["chat", "tabs.list", "tabs.activate", "page.snapshot"],
    }),
  );
  console.log("→ extension connected, hello sent");

  // B) the JSON-RPC client. Small delay so `hello` is ingested first.
  await new Promise((r) => setTimeout(r, 300));
  const client = await open("client");
  console.log("→ client connected");

  // 1. status should show the extension connected.
  let status = await rpc(client, "chrome/status", {});
  console.log("chrome/status:", JSON.stringify(status));
  if (!status || status.connected !== true) fail("chrome/status did not report connected");
  if (!Array.isArray(status.capabilities) || !status.capabilities.includes("tabs.list"))
    fail("chrome/status did not surface the advertised capabilities");

  // 2. enable chrome tools for the session.
  status = await rpc(client, "chrome/enable", { mode: "assist" });
  if (!status.enabled) fail("chrome/enable did not enable chrome");
  console.log("chrome/enable: enabled, mode =", status.mode);

  // 3. tabs/list should round-trip through the extension.
  const tabs = await rpc(client, "chrome/tabs/list", {});
  console.log("chrome/tabs/list:", JSON.stringify(tabs));
  const list = tabs.tabs ?? tabs;
  if (!Array.isArray(list) || list.length !== FAKE_TABS.length)
    fail("chrome/tabs/list did not return the extension's tabs");
  if (list[0].title !== "Example Domain") fail("tab payload was not delivered verbatim");

  // 4. a page snapshot should round-trip too.
  const snap = await rpc(client, "chrome/page/snapshot", { tabId: 1 });
  console.log("chrome/page/snapshot:", JSON.stringify(snap));

  console.log("\nPASS: extension ⇄ app-server bridge ⇄ chrome/* methods round-trip end to end.");
  ext.close();
  client.close();
  process.exit(0);
}

main().catch((err) => fail(err.message));
