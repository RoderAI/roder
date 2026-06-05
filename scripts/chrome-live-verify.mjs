#!/usr/bin/env node
// Live end-to-end check: make the REAL unpacked MV3 extension (loaded in a
// dedicated Chrome via --load-extension) pair with a running
// `roder app-server --remote`, then confirm from the SERVER side that the
// extension connected and that real browser tabs round-trip through the bridge.
//
// Strategy: the MV3 service worker is privileged (has chrome.storage). We wake
// it by navigating a tab (fires tabs.onUpdated), attach via CDP, seed the
// connection settings into chrome.storage, then chrome.runtime.reload() so the
// extension's init() auto-connects on restart.
//
// Usage: node chrome-live-verify.mjs <debugPort> <wsEndpoint> <token> <extId>
import { setTimeout as sleep } from "node:timers/promises";

const [, , debugPort, endpoint, token, extId] = process.argv;
if (!debugPort || !endpoint || !token || !extId) {
  console.error("usage: chrome-live-verify.mjs <debugPort> <wsEndpoint> <token> <extId>");
  process.exit(2);
}
const base = `http://127.0.0.1:${debugPort}`;

function connectWs(url, protocols) {
  return new Promise((resolve, reject) => {
    const ws = protocols ? new WebSocket(url, protocols) : new WebSocket(url);
    ws.addEventListener("open", () => resolve(ws));
    ws.addEventListener("error", (e) => reject(new Error(e.message ?? "ws error")));
  });
}
function attach(ws) {
  let id = 0;
  const pend = new Map();
  ws.addEventListener("message", (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && pend.has(m.id)) {
      const { res, rej } = pend.get(m.id);
      pend.delete(m.id);
      m.error ? rej(new Error(JSON.stringify(m.error))) : res(m.result);
    }
  });
  return (method, params = {}) => {
    const i = ++id;
    ws.send(JSON.stringify({ id: i, method, params }));
    return new Promise((res, rej) => pend.set(i, { res, rej }));
  };
}
const fail = (m) => {
  console.error(`FAIL: ${m}`);
  process.exit(1);
};
const targets = () => fetch(`${base}/json`).then((r) => r.json());

async function main() {
  // 1. Navigate a tab to a real page: gives us a real tab AND wakes the SW.
  const tlist = await targets();
  const page = tlist.find((t) => t.type === "page");
  if (!page) fail("no page target to drive");
  const pws = attach(await connectWs(page.webSocketDebuggerUrl));
  await pws("Page.enable");
  await pws("Page.navigate", { url: "https://example.com/" });
  console.log("→ navigated a tab to https://example.com (wakes the service worker)");

  // 2. Find the extension's service worker target.
  let sw;
  for (let i = 0; i < 25 && !sw; i++) {
    await sleep(400);
    sw = (await targets()).find(
      (t) => t.type === "service_worker" && (t.url ?? "").includes(extId),
    );
  }
  if (!sw) fail("extension service worker never appeared (extension may not have loaded)");
  console.log(`→ service worker awake: ${sw.url}`);

  // 3. Seed settings + reload so init() auto-connects.
  const sws = attach(await connectWs(sw.webSocketDebuggerUrl));
  await sws("Runtime.enable");
  const settings = {
    endpoint,
    token,
    autoConnect: true,
    controlMode: "assist",
    allowPageInspection: true,
    allowNavigation: false,
    allowInput: false,
  };
  const seed = await sws("Runtime.evaluate", {
    expression: `(async () => {
      if (!chrome?.storage?.local) return "NO_STORAGE";
      await chrome.storage.local.set({ 'roderChrome.settings': ${JSON.stringify(settings)} });
      const id = chrome.runtime.id;
      setTimeout(() => chrome.runtime.reload(), 50);
      return id;
    })()`,
    awaitPromise: true,
    returnByValue: true,
  });
  if (seed.exceptionDetails) fail(`SW eval threw: ${JSON.stringify(seed.exceptionDetails)}`);
  if (seed.result.value === "NO_STORAGE") fail("service worker context lacked chrome.storage");
  console.log(`→ seeded settings into extension ${seed.result.value}; reloading to auto-connect`);

  // 4. Confirm from the server side.
  const client = await connectWs(endpoint, ["roder.remote.v1", `bearer.${token}`]);
  let rpcId = 0;
  const rpc = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = `c-${++rpcId}`;
      const onMsg = (ev) => {
        const m = JSON.parse(ev.data);
        if (m.id !== id) return;
        client.removeEventListener("message", onMsg);
        m.error ? reject(new Error(m.error.message)) : resolve(m.result);
      };
      client.addEventListener("message", onMsg);
      client.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
      setTimeout(() => {
        client.removeEventListener("message", onMsg);
        reject(new Error(`${method} timeout`));
      }, 8000);
    });

  let status;
  for (let i = 0; i < 30; i++) {
    status = await rpc("chrome/status");
    if (status.connected) break;
    await sleep(500);
  }
  console.log("chrome/status:", JSON.stringify(status));
  if (!status.connected) fail("the real extension never registered with the app-server");
  console.log(`✓ REAL MV3 extension connected — capabilities: ${status.capabilities.join(", ")}`);

  await rpc("chrome/enable", { mode: "assist" });
  const tabs = await rpc("chrome/tabs/list");
  const list = tabs.tabs ?? tabs;
  console.log(`chrome/tabs/list → ${Array.isArray(list) ? list.length : "?"} real tab(s):`);
  for (const t of list ?? []) console.log(`   [${t.id}] ${t.title ?? "(untitled)"} — ${t.url ?? ""}`);

  console.log("\nLIVE PASS: real Chrome extension ⇄ roder app-server bridge ⇄ chrome/* methods.");
  client.close();
  process.exit(0);
}

main().catch((e) => fail(e.message ?? String(e)));
