#!/usr/bin/env node
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

if (process.env.RODER_SDK_LIVE !== "1") {
  console.log("skipped: set RODER_SDK_LIVE=1 to run the TypeScript live smoke");
  process.exit(0);
}

const command = process.env.RODER_BIN ?? "cargo";
const args = process.env.RODER_BIN
  ? ["app-server", "--listen", "stdio://"]
  : ["run", "-p", "roder", "--bin", "roder", "--", "app-server", "--listen", "stdio://"];

if (process.env.RODER_REMOTE_URL && process.env.RODER_REMOTE_TOKEN) {
  const response = await remoteCall(process.env.RODER_REMOTE_URL, process.env.RODER_REMOTE_TOKEN, {
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {},
  });
  console.log(`typescript remote live smoke ok: ${response.result?.provider ?? "provider"}`);
  process.exit(0);
}

const child = spawn(command, args, { stdio: "pipe" });
const lines = createInterface({ input: child.stdout });
const pending = new Map();
let nextId = 1;

lines.on("line", (line) => {
  const message = JSON.parse(line);
  if ("id" in message) {
    const resolve = pending.get(String(message.id));
    pending.delete(String(message.id));
    resolve?.(message);
  }
});

child.stderr.on("data", (chunk) => process.stderr.write(chunk));

try {
  const init = await call("initialize", {});
  const thread = await call("thread/start", {
    cwd: process.cwd(),
    modelProvider: "mock",
    model: "mock",
  });
  const threadId = thread.result?.thread?.id ?? thread.result?.threadId ?? thread.result?.id;
  const turn = await call("turn/start", {
    threadId,
    input: [{ type: "text", text: "live smoke" }],
  });
  const turnId = turn.result?.turn?.id ?? turn.result?.turnId ?? turn.result?.id;
  await call("turn/interrupt", { threadId, turnId, reason: "live smoke complete" });
  console.log(`typescript live smoke ok: ${init.result?.provider ?? "provider"} ${threadId}`);
} finally {
  child.kill();
}

function call(method, params) {
  const id = nextId++;
  const request = { jsonrpc: "2.0", id, method, params };
  const promise = new Promise((resolve, reject) => {
    pending.set(String(id), (message) => {
      if (message.error) {
        reject(new Error(`${method}: ${message.error.message}`));
      } else {
        resolve(message);
      }
    });
  });
  child.stdin.write(`${JSON.stringify(request)}\n`);
  return promise;
}

function remoteCall(url, token, request) {
  const socket = new WebSocket(url, [], { headers: { Authorization: `Bearer ${token}` } });
  return new Promise((resolve, reject) => {
    socket.addEventListener("open", () => socket.send(JSON.stringify(request)));
    socket.addEventListener("error", reject);
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(String(event.data));
      socket.close();
      if (message.error) {
        reject(new Error(message.error.message));
      } else {
        resolve(message);
      }
    });
  });
}
