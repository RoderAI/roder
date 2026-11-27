import { createInterface } from "node:readline";
import { spawn } from "node:child_process";

const mode = process.argv[2];
const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
let callbackRequest;
let descendant;
if (mode === "descendant") {
  descendant = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
  // Reap a normally terminated child while the root is still alive.
  process.on("SIGTERM", () => {});
  descendant.on("exit", () => process.exit(0));
}
if (mode === "stubborn") {
  process.on("SIGTERM", () => {});
  setInterval(() => {}, 1000);
}
const input = createInterface({ input: process.stdin });
if (mode === "graceful") {
  process.on("SIGTERM", () => process.exit(9));
  input.on("close", () => {
    setTimeout(() => process.stderr.write("graceful EOF complete\n"), 120);
  });
}
input.on("line", (line) => {
  const message = JSON.parse(line);
  if (!message.method) {
    if (callbackRequest) {
      send({ jsonrpc: "2.0", id: callbackRequest, result: message });
      callbackRequest = undefined;
    }
    return;
  }
  const reply = (result) => send({ jsonrpc: "2.0", id: message.id, result });
  switch (message.method) {
    case "initialize":
      reply({ protocolVersion: 1, agentCapabilities: {}, received: message.params });
      break;
    case "echo":
      setTimeout(() => reply(message.params), message.params.delay ?? 0);
      break;
    case "never":
      break;
    case "descendant-pid":
      reply(descendant?.pid ?? null);
      break;
    case "error":
      send({ jsonrpc: "2.0", id: message.id, error: { code: -32001, message: "fixture error", data: "detail" } });
      break;
    case "notification":
      send({ jsonrpc: "2.0", method: "session/update", params: message.params });
      break;
    case "callback":
      callbackRequest = message.id;
      send({ jsonrpc: "2.0", id: "permission", method: "session/request_permission", params: message.params });
      break;
    case "malformed":
      process.stdout.write("not json\n");
      break;
    case "invalid-envelope":
      process.stdout.write('{"jsonrpc":"2.0","id":1,"result":{},"error":{}}\n');
      break;
    case "invalid-utf8":
      process.stdout.write(Buffer.from([123, 34, 120, 34, 58, 34, 255, 34, 125, 10]));
      break;
    case "oversized":
      process.stdout.write("x".repeat(4096));
      break;
    case "fragmented": {
      const data = Buffer.from(`${JSON.stringify({ jsonrpc: "2.0", id: message.id, result: "hello \u{1f680}" })}\n`);
      const split = data.indexOf(Buffer.from("\u{1f680}")) + 1;
      process.stdout.write(data.subarray(0, split));
      setTimeout(() => process.stdout.write(data.subarray(split)), 5);
      break;
    }
    case "stderr":
      process.stderr.write("x".repeat(100_000), () => reply(process.env.RODER_TRANSPORT_TEST));
      break;
    case "exit":
      process.exit(7);
      break;
    default:
      reply(null);
  }
});
