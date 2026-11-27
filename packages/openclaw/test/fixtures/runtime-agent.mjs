import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { join } from "node:path";
import { createInterface } from "node:readline";

const directory = process.argv[2];
const options = JSON.parse(process.argv[3] ?? "{}");
const log = (value) => appendFileSync(join(directory, "calls.jsonl"), `${JSON.stringify({ pid: process.pid, ...value })}\n`);
const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
const reply = (id, result) => send({ jsonrpc: "2.0", id, result });
const filename = (id) => join(directory, `native-${id}.json`);
const save = (state) => writeFileSync(filename(state.sessionId), JSON.stringify(state));
const load = (id) => {
  if (!/^[a-f0-9-]+$/.test(id) || !existsSync(filename(id))) throw new Error("Unknown native session");
  return JSON.parse(readFileSync(filename(id), "utf8"));
};
const controls = (state) => ({
  sessionId: state.sessionId,
  modes: { currentModeId: state.mode, availableModes: [{ id: "default", name: "Default" }, { id: "plan", name: "Plan" }] },
  configOptions: [{
    id: "model", name: "Model", type: "select", currentValue: state.model,
    options: [{ group: "Fixture", options: [{ value: "fixture/fast", name: "Fast" }, { value: "fixture/deep", name: "Deep" }] }],
  }],
});
let active;
const update = (sessionId, value) => send({ jsonrpc: "2.0", method: "session/update", params: { sessionId, update: value } });
const complete = () => {
  if (!active) return;
  reply(active.id, { stopReason: "end_turn" });
  active = undefined;
};

log({ method: "fixture/started" });
createInterface({ input: process.stdin }).on("line", (line) => {
  const message = JSON.parse(line);
  log(message);
  if (!message.method) {
    if (message.id === "permission") complete();
    return;
  }
  const { id, method, params } = message;
  try {
    switch (method) {
      case "initialize":
        if (options.initializeWait) break;
        reply(id, {
          protocolVersion: 1,
          agentCapabilities: {
            sessionCapabilities: {
              ...(options.resume === false ? {} : { resume: true }),
              ...(options.close === false ? {} : { close: true }),
            },
            promptCapabilities: { image: options.image !== false },
            mcpCapabilities: { http: options.http !== false },
          },
        });
        break;
      case "session/new": {
        const state = { sessionId: randomUUID(), prompts: [], mode: "default", model: "fixture/fast" };
        save(state);
        reply(id, controls(state));
        break;
      }
      case "session/resume":
        reply(id, controls(load(params.sessionId)));
        break;
      case "session/set_mode": {
        const state = load(params.sessionId);
        state.mode = params.modeId;
        save(state);
        reply(id, {});
        break;
      }
      case "session/set_config_option": {
        if (options.configWait) break;
        const state = load(params.sessionId);
        state.model = params.value;
        save(state);
        reply(id, { configOptions: controls(state).configOptions });
        break;
      }
      case "session/prompt": {
        const state = load(params.sessionId);
        const text = params.prompt.find((block) => block.type === "text").text;
        state.prompts.push(params.prompt);
        save(state);
        if (text === "rpc-failure") {
          send({ jsonrpc: "2.0", id, error: { code: -32000, message: "Provider authentication required" } });
          break;
        }
        active = { id, sessionId: state.sessionId, stubborn: text === "uncooperative" };
        if (active.stubborn) {
          process.on("SIGTERM", () => {});
          setInterval(() => {}, 1000);
        }
        update(state.sessionId, { sessionUpdate: "agent_message_chunk", content: { type: "text", text: `turn ${state.prompts.length}` } });
        if (text === "wait" || text === "uncooperative") break;
        update(state.sessionId, { sessionUpdate: "agent_thought_chunk", content: { type: "text", text: "thinking" } });
        update(state.sessionId, { sessionUpdate: "tool_call", toolCallId: "read-1", title: "Read fixture", kind: "read", status: "in_progress" });
        update(state.sessionId, { sessionUpdate: "tool_call_update", toolCallId: "read-1", status: "completed", content: [
          { type: "content", content: { type: "text", text: "fixture contents" } },
          { type: "diff", path: "fixture.txt", newText: "new fixture" },
        ] });
        update(state.sessionId, { sessionUpdate: "plan", entries: [{ status: "completed", content: "Verify fixture" }] });
        update(state.sessionId, { sessionUpdate: "usage_update", used: 42, size: 1000 });
        if (text.startsWith("permission:")) {
          send({ jsonrpc: "2.0", id: "permission", method: "session/request_permission", params: {
            sessionId: state.sessionId,
            toolCall: { kind: text.split(":")[1], toolCallId: "permission-tool" },
            options: [{ kind: "allow_once", optionId: "yes" }, { kind: "reject_once", optionId: "no" }],
          } });
        } else complete();
        break;
      }
      case "session/cancel":
        if (active && !active.stubborn) {
          reply(active.id, { stopReason: "cancelled" });
          active = undefined;
        }
        break;
      case "session/close":
        reply(id, {});
        break;
      default:
        send({ jsonrpc: "2.0", id, error: { code: -32601, message: "Unsupported fixture method" } });
    }
  } catch (error) {
    send({ jsonrpc: "2.0", id, error: { code: -32603, message: error.message } });
  }
});
