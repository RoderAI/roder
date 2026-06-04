#!/usr/bin/env node
// Validates the 1-click auto-pair flow with the REAL extension, no SW poking:
// navigate a tab to the app-server's /pair URL (token in the fragment); the
// extension's declared content script reads it, messages the service worker,
// and the worker connects. Then confirm from the server side and browse a site
// through the bridge.
//
// Usage: node chrome-oneclick-verify.mjs <debugPort> <port> <token>
import { setTimeout as sleep } from "node:timers/promises";

const [, , debugPort, port, token] = process.argv;
const endpoint = `ws://127.0.0.1:${port}`;
const b64 = Buffer.from(JSON.stringify({ endpoint, token })).toString("base64url");
const pairUrl = `http://127.0.0.1:${port}/pair#roder-pair=${b64}`;

function connectWs(url, protocols) {
  return new Promise((res, rej) => {
    const ws = protocols ? new WebSocket(url, protocols) : new WebSocket(url);
    ws.addEventListener("open", () => res(ws));
    ws.addEventListener("error", (e) => rej(new Error(e.message ?? "ws error")));
  });
}
function attach(ws) {
  let id = 0; const pend = new Map();
  ws.addEventListener("message", (ev) => { const m = JSON.parse(ev.data); if (m.id && pend.has(m.id)) { pend.get(m.id)(m.result); pend.delete(m.id); } });
  return (method, params = {}) => { const i = ++id; ws.send(JSON.stringify({ id: i, method, params })); return new Promise((r) => pend.set(i, r)); };
}
const fail = (m) => { console.error(`FAIL: ${m}`); process.exit(1); };
const targets = () => fetch(`http://127.0.0.1:${debugPort}/json`).then((r) => r.json());

async function main() {
  console.log(`1-click pair URL:\n  ${pairUrl}\n`);
  // Open the pair URL in a tab → content script auto-pairs.
  const page = (await targets()).find((t) => t.type === "page");
  const pcdp = attach(await connectWs(page.webSocketDebuggerUrl));
  await pcdp("Page.enable");
  await pcdp("Page.navigate", { url: pairUrl });
  console.log("→ opened the 1-click pair URL; extension content script should connect…");

  // Confirm from the server side.
  const client = await connectWs(endpoint, ["roder.remote.v1", `bearer.${token}`]);
  let rid = 0;
  const rpc = (method, params = {}) => new Promise((resolve, reject) => {
    const id = `c-${++rid}`;
    const onMsg = (ev) => { const m = JSON.parse(ev.data); if (m.id !== id) return; client.removeEventListener("message", onMsg); m.error ? reject(new Error(m.error.message)) : resolve(m.result); };
    client.addEventListener("message", onMsg);
    client.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    setTimeout(() => { client.removeEventListener("message", onMsg); reject(new Error(`${method} timeout`)); }, 8000);
  });

  let status;
  for (let i = 0; i < 30; i++) { status = await rpc("chrome/status"); if (status.connected) break; await sleep(500); }
  if (!status.connected) fail("extension did not auto-connect from the 1-click URL");
  console.log(`✓ auto-connected via 1-click — capabilities: ${status.capabilities.join(", ")}`);
  await rpc("chrome/enable", { mode: "assist" });

  // Browse a real website through the plugin: open example.com, read it back.
  const ver = await (await fetch(`http://127.0.0.1:${debugPort}/json/version`)).json();
  const bcdp = attach(await connectWs(ver.webSocketDebuggerUrl));
  await bcdp("Target.createTarget", { url: "https://example.com/" });
  await sleep(1500);

  const tabs = await rpc("chrome/tabs/list");
  const list = tabs.tabs ?? tabs;
  console.log(`chrome/tabs/list → ${list.length} tab(s):`);
  for (const t of list) console.log(`   [${t.id}] ${t.title ?? ""} — ${t.url ?? ""}`);
  const site = list.find((t) => (t.url ?? "").includes("example.com")) ?? list[list.length - 1];

  const snap = await rpc("chrome/page/snapshot", { tabId: site.id });
  const content = snap.content ?? snap;
  console.log(`\nchrome/page/snapshot of tab ${site.id} (UNTRUSTED page content via the plugin):`);
  console.log(`   title: ${content.title}`);
  console.log(`   url:   ${content.url}`);
  console.log(`   text:  ${String(content.text ?? "").slice(0, 80)}…`);
  console.log(`   untrusted flag present: ${snap.untrusted === true}`);

  console.log("\nONE-CLICK PASS: pair URL → content-script auto-connect → browsed a website through the plugin.");
  client.close();
  process.exit(0);
}
main().catch((e) => fail(e.message ?? String(e)));
