import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { parseConfig } from "../src/config.mjs";
import { RoderRuntime } from "../src/runtime.mjs";

const fixture = fileURLToPath(new URL("./fixtures/runtime-agent.mjs", import.meta.url));
const owner = { sessionKey: "chat:shared", agentId: "owner-a", agent: "roder", mode: "persistent" };
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

test("standalone runtime applies the same validated defaults as the plugin", async () => {
  const runtime = new RoderRuntime({ stateDir: join(tmpdir(), "unused-roder-runtime-defaults") });
  assert.equal(runtime.config.command, "roder");
  assert.deepEqual(runtime.config.args, ["acp"]);
  assert.deepEqual(runtime.config.mcpServers, []);
  assert.equal(runtime.config.permissionMode, "deny-all");
  await runtime.stop();
  assert.throws(() => new RoderRuntime({ stateDir: "/tmp/state", unknown: true }), /unknown option/);
});

async function setup(t, overrides = {}, fixtureOptions = {}) {
  const root = await mkdtemp(join(tmpdir(), "roder-runtime-test-"));
  const dataDir = join(root, "native");
  await mkdir(dataDir);
  const config = {
    ...parseConfig({
      command: process.execPath, args: [fixture, dataDir, JSON.stringify(fixtureOptions)],
      cwd: root, cancelTimeoutMs: 100, ...overrides,
    }),
    stateDir: join(root, "plugin"),
  };
  const runtimes = [];
  const create = () => {
    const runtime = new RoderRuntime(config);
    runtimes.push(runtime);
    return runtime;
  };
  t.after(async () => {
    await Promise.all(runtimes.map((runtime) => runtime.stop()));
    await rm(root, { recursive: true, force: true });
  });
  const calls = async () => {
    try { return (await readFile(join(dataDir, "calls.jsonl"), "utf8")).trim().split("\n").map(JSON.parse); }
    catch (error) { if (error.code === "ENOENT") return []; throw error; }
  };
  const native = async (handle) => JSON.parse(await readFile(join(dataDir, `native-${handle.backendSessionId}.json`), "utf8"));
  return { runtime: create(), create, calls, native, config };
}

async function collect(runtime, handle, text = "hello", extra = {}) {
  return Array.fromAsync(runtime.runTurn({ handle, text, mode: "prompt", ...extra }));
}

async function waitFor(fn, label) {
  for (let attempt = 0; attempt < 300; attempt++) {
    const value = await fn();
    if (value) return value;
    await sleep(10);
  }
  assert.fail(`Timed out waiting for ${label}`);
}

function gone(pid) {
  try { process.kill(pid, 0); return false; }
  catch (error) { if (error.code === "ESRCH") return true; throw error; }
}

test("streams ACP text, thought, tool, plan and usage events with a completed terminal", async (t) => {
  const { runtime } = await setup(t);
  const handle = await runtime.ensureSession(owner);
  const events = await collect(runtime, handle);
  assert.ok(events.some((event) => event.type === "text_delta" && event.stream === "output" && event.text === "turn 1"));
  assert.ok(events.some((event) => event.type === "text_delta" && event.stream === "thought" && event.text === "thinking"));
  assert.ok(events.some((event) => event.type === "tool_call" && event.toolCallId === "read-1" && event.kind === "read"));
  assert.ok(events.some((event) => event.tag === "tool_call_update" && event.text.includes("fixture contents") && event.text.includes("fixture.txt")));
  assert.ok(events.some((event) => event.tag === "plan" && event.text.includes("Verify fixture")));
  assert.ok(events.some((event) => event.tag === "usage_update" && event.used === 42 && event.size === 1000));
  assert.deepEqual(events.at(-1), { type: "done", status: "completed", stopReason: "end_turn" });
});

test("cold resumes durable native state and isolates owners sharing a session key", async (t) => {
  const { runtime, create, calls } = await setup(t);
  const first = await runtime.ensureSession(owner);
  await collect(runtime, first);
  await runtime.stop();
  const restarted = create();
  const resumed = await restarted.ensureSession({ ...owner, persistedHandle: first });
  assert.equal(resumed.backendSessionId, first.backendSessionId);
  assert.equal(resumed.runtimeSessionName, first.runtimeSessionName);
  assert.ok((await collect(restarted, resumed)).some((event) => event.text === "turn 2"));
  const other = await restarted.ensureSession({ ...owner, agentId: "owner-b" });
  assert.notEqual(other.backendSessionId, resumed.backendSessionId);
  assert.ok((await collect(restarted, other)).some((event) => event.text === "turn 1"));
  const recorded = await calls();
  assert.equal(recorded.filter((call) => call.method === "session/new").length, 2);
  assert.equal(recorded.filter((call) => call.method === "session/resume").length, 1);
  await assert.rejects(restarted.getStatus({ handle: { ...first, agentId: "unknown-owner" } }), /Stale|unknown/);
});

test("close preserves persistent native state; discard and reset invalidate old handles", async (t) => {
  const { runtime, native, calls } = await setup(t);
  const first = await runtime.ensureSession(owner);
  await collect(runtime, first);
  await runtime.close({ handle: first });
  const resumed = await runtime.ensureSession(owner);
  assert.equal(resumed.backendSessionId, first.backendSessionId);
  assert.equal((await native(resumed)).prompts.length, 1);
  await runtime.close({ handle: resumed, discardPersistentState: true });
  const fresh = await runtime.ensureSession(owner);
  assert.notEqual(fresh.backendSessionId, first.backendSessionId);
  await assert.rejects(runtime.getStatus({ handle: first }), /Stale|unknown/);
  await runtime.prepareFreshSession(owner);
  const reset = await runtime.ensureSession(owner);
  assert.notEqual(reset.runtimeSessionName, fresh.runtimeSessionName);
  await assert.rejects(runtime.cancel({ handle: fresh }), /Stale|unknown/);
  assert.equal((await calls()).filter((call) => call.method === "session/close").length, 2);
});

test("oneshot close starts fresh while persistent missing mappings cannot silently recover", async (t) => {
  const { runtime } = await setup(t);
  const input = { ...owner, mode: "oneshot" };
  const first = await runtime.ensureSession(input);
  await runtime.close({ handle: first });
  await assert.rejects(runtime.ensureSession({ ...input, persistedHandle: first }), /mapping is missing/);
  const second = await runtime.ensureSession(input);
  assert.notEqual(second.backendSessionId, first.backendSessionId);
});

test("validates explicit models, drops unsupported inherited defaults, and applies supported models", async (t) => {
  const { runtime, calls } = await setup(t);
  await assert.rejects(runtime.ensureSession({ ...owner, model: "unknown", modelExplicit: true }), { code: "ACP_INVALID_RUNTIME_OPTION" });
  const dropped = await runtime.ensureSession({ ...owner, model: "unknown", modelExplicit: false });
  assert.deepEqual(dropped.appliedModel, { kind: "dropped" });
  const handle = await runtime.ensureSession({ ...owner, model: "fixture/deep", modelExplicit: true });
  const status = await runtime.getStatus({ handle });
  assert.equal(status.details.configOptions[0].currentValue, "fixture/deep");
  assert.equal((await calls()).filter((call) => call.method === "session/set_config_option").length, 1);
  await assert.rejects(runtime.ensureSession({ ...owner, model: "" }), { code: "ACP_INVALID_RUNTIME_OPTION" });
});

test("rejects environment, explicit thinking and steer while dropping inherited thinking", async (t) => {
  const { runtime, calls } = await setup(t);
  await assert.rejects(runtime.ensureSession({ ...owner, env: { SECRET: "not-forwarded" } }), { code: "ACP_INVALID_RUNTIME_OPTION" });
  await assert.rejects(runtime.ensureSession({ ...owner, thinking: "high" }), { code: "ACP_INVALID_RUNTIME_OPTION" });
  assert.equal((await calls()).length, 0);
  const handle = await runtime.ensureSession({ ...owner, thinking: "high", thinkingExplicit: false });
  assert.deepEqual(handle.appliedThinking, { kind: "dropped" });
  const events = await collect(runtime, handle, "steer", { mode: "steer" });
  assert.equal(events.at(-1).code, "ACP_BACKEND_UNSUPPORTED_CONTROL");
  assert.equal((await calls()).filter((call) => call.method === "session/prompt").length, 0);
});

test("roundtrips image attachments, rejects invalid base64, and checks advertised capabilities", async (t) => {
  const { runtime, calls } = await setup(t);
  const handle = await runtime.ensureSession(owner);
  await collect(runtime, handle, "describe", { attachments: [{ mediaType: "image/png", data: "aGVsbG8=" }] });
  const prompt = (await calls()).find((call) => call.method === "session/prompt").params.prompt;
  assert.deepEqual(prompt, [{ type: "text", text: "describe" }, { type: "image", mimeType: "image/png", data: "aGVsbG8=" }]);
  assert.equal((await collect(runtime, handle, "invalid", { attachments: [{ mediaType: "image/png", data: "!" }] })).at(-1).type, "error");
  const noImages = await setup(t, {}, { image: false });
  const other = await noImages.runtime.ensureSession(owner);
  const rejected = await collect(noImages.runtime, other, "image", { attachments: [{ mediaType: "image/png", data: "aGVsbG8=" }] });
  assert.equal(rejected.at(-1).code, "ACP_INVALID_RUNTIME_OPTION");
  assert.equal((await noImages.calls()).filter((call) => call.method === "session/prompt").length, 0);
});

test("advertises controls, applies modes/configuration, and rejects controls while busy", async (t) => {
  const { runtime } = await setup(t);
  const handle = await runtime.ensureSession(owner);
  const capabilities = await runtime.getCapabilities({ handle });
  assert.deepEqual(capabilities.controls, ["session/set_mode", "session/set_config_option", "session/status"]);
  await runtime.setMode({ handle, mode: "plan" });
  await runtime.setConfigOption({ handle, key: "model", value: "fixture/deep" });
  assert.equal((await runtime.getStatus({ handle })).details.mode, "plan");
  await assert.rejects(runtime.setMode({ handle, mode: "unknown" }), { code: "ACP_INVALID_RUNTIME_OPTION" });
  const turn = runtime.startTurn({ handle, mode: "prompt", text: "wait" });
  await turn.promptStarted;
  assert.equal((await runtime.getStatus({ handle })).summary, "Roder is running");
  await assert.rejects(runtime.setMode({ handle, mode: "default" }), { code: "ACP_INVALID_RUNTIME_OPTION" });
  await assert.rejects(runtime.setConfigOption({ handle, key: "model", value: "fixture/fast" }), { code: "ACP_INVALID_RUNTIME_OPTION" });
  const concurrent = runtime.startTurn({ handle, mode: "prompt", text: "hello" });
  assert.equal((await concurrent.result).error.code, "ACP_TURN_FAILED");
  await turn.cancel();
  assert.equal((await turn.result).status, "cancelled");
  assert.equal((await runtime.getStatus({ handle })).summary, "Roder is idle");
});

for (const [policy, kind, optionId] of [
  ["deny-all", "read", "no"], ["approve-reads", "read", "yes"],
  ["approve-reads", "edit", "no"], ["approve-all", "edit", "yes"],
]) {
  test(`permission policy ${policy} selects ${optionId} for ${kind}`, async (t) => {
    const { runtime, calls } = await setup(t, { permissionMode: policy });
    const handle = await runtime.ensureSession(owner);
    assert.equal((await collect(runtime, handle, `permission:${kind}`)).at(-1).status, "completed");
    const response = (await calls()).find((call) => call.id === "permission" && !call.method);
    assert.deepEqual(response.result, { outcome: { outcome: "selected", optionId } });
  });
}

test("JSON-RPC prompt failure produces an authentication error event without destroying the session", async (t) => {
  const { runtime } = await setup(t);
  const handle = await runtime.ensureSession(owner);
  const events = await collect(runtime, handle, "rpc-failure");
  assert.deepEqual(events.at(-1), {
    type: "error", code: "ACP_TURN_FAILED", detailCode: "AUTH_REQUIRED", message: "Provider authentication required",
  });
  assert.equal((await collect(runtime, handle)).at(-1).status, "completed");
});

test("abort signals cancel turns, and uncooperative prompts force child termination before resume", async (t) => {
  const { runtime, calls } = await setup(t);
  const handle = await runtime.ensureSession(owner);
  const controller = new AbortController();
  const turn = runtime.startTurn({ handle, mode: "prompt", text: "uncooperative", signal: controller.signal });
  await turn.promptStarted;
  const firstPid = (await waitFor(async () => (await calls()).find((call) => call.method === "session/prompt"), "prompt")).pid;
  controller.abort();
  assert.equal((await turn.result).status, "cancelled");
  await waitFor(() => gone(firstPid), "cancelled child exit");
  assert.equal((await collect(runtime, handle)).at(-1).status, "completed");
  assert.equal((await calls()).filter((call) => call.method === "session/resume").length, 1);
});

test("early stream consumers cancel the prompt, and runtime.stop cleans active children", async (t) => {
  const { runtime, calls } = await setup(t);
  const handle = await runtime.ensureSession(owner);
  for await (const event of runtime.runTurn({ handle, mode: "prompt", text: "wait" })) {
    assert.equal(event.type, "text_delta");
    break;
  }
  assert.ok((await calls()).some((call) => call.method === "session/cancel"));
  const turn = runtime.startTurn({ handle, mode: "prompt", text: "wait" });
  await turn.promptStarted;
  await runtime.stop();
  assert.equal((await turn.result).status, "cancelled");
  const pids = new Set((await calls()).map((call) => call.pid));
  for (const pid of pids) await waitFor(() => gone(pid), `stopped child ${pid}`);
  await assert.rejects(runtime.ensureSession(owner), { code: "ACP_BACKEND_UNAVAILABLE" });
});

test("pre-aborted turns create no prompt and doctor probes transport without authenticating", async (t) => {
  const { runtime, calls } = await setup(t);
  const controller = new AbortController();
  controller.abort();
  const turn = runtime.startTurn({ handle: {}, mode: "prompt", text: "hello", signal: controller.signal });
  assert.equal((await turn.result).status, "cancelled");
  const doctor = await runtime.doctor();
  assert.equal(doctor.ok, true);
  assert.match(doctor.details.join(" "), /authentication.*separate prompt/);
  const recorded = await calls();
  assert.equal(recorded.filter((call) => call.method === "initialize").length, 1);
  assert.equal(recorded.filter((call) => call.method === "session/new" || call.method === "session/prompt").length, 0);
  for (const pid of new Set(recorded.map((call) => call.pid))) await waitFor(() => gone(pid), "doctor child cleanup");
});

test("requires lifecycle capabilities and never creates a session on incompatible binaries", async (t) => {
  for (const capability of ["resume", "close"]) {
    const { runtime, calls } = await setup(t, {}, { [capability]: false });
    assert.equal((await runtime.doctor()).ok, false);
    await assert.rejects(runtime.ensureSession(owner), /lacks session\/resume or session\/close/);
    assert.ok(!(await calls()).some((call) => call.method === "session/new"));
  }
});

test("rejects unmapped resume ids and invalid inputs before spawning any executable", async (t) => {
  const { runtime, calls } = await setup(t);
  for (const patch of [
    { resumeSessionId: "foreign-native-id" }, { resumeSessionId: "" },
    { cwd: "relative" }, { cwd: "/tmp/invalid\0cwd" },
    { env: [] }, { env: null }, { model: "" }, { model: null },
    { agent: "not-roder" }, { mode: "invalid" },
  ]) await assert.rejects(runtime.ensureSession({ ...owner, ...patch }));
  assert.deepEqual(await calls(), []);
});

test("launch/MCP changes fail closed instead of rebinding stored sessions", async (t) => {
  const { runtime, config, calls } = await setup(t);
  const handle = await runtime.ensureSession(owner);
  await runtime.stop();
  const changed = new RoderRuntime({
    ...config, mcpServers: [{ name: "new-tools", command: "no-such-server", args: [], env: [] }],
  });
  t.after(() => changed.stop());
  await assert.rejects(changed.ensureSession(owner), /differs from requested/);
  await assert.rejects(changed.getStatus({ handle }), /configuration changed/);
  assert.equal((await calls()).filter((call) => call.method === "session/resume").length, 0);
});

test("injects the exact configured MCP bindings on new and cold resume", async (t) => {
  const mcpServers = [
    { name: "stdio-tools", command: "fixture-server", args: ["--stdio"], env: [{ name: "TOKEN", value: "private-env" }] },
    { type: "http", name: "http-tools", url: "https://example.test/mcp", headers: [{ name: "Authorization", value: "private-header" }] },
  ];
  const { runtime, create, calls } = await setup(t, { mcpServers });
  const handle = await runtime.ensureSession(owner);
  await runtime.stop();
  const restored = create();
  await restored.ensureSession({ ...owner, persistedHandle: handle, resumeSessionId: handle.backendSessionId });
  for (const call of (await calls()).filter((call) => ["session/new", "session/resume"].includes(call.method))) {
    assert.deepEqual(call.params.mcpServers, mcpServers);
  }
  assert.equal(restored.turnError(new Error("private-env private-header")).message, "[redacted] [redacted]");
});

test("HTTP injection fails before session creation when the agent lacks the capability", async (t) => {
  const { runtime, calls } = await setup(t, {
    mcpServers: [{ type: "http", name: "tools", url: "https://example.test/mcp", headers: [] }],
  }, { http: false });
  await assert.rejects(runtime.ensureSession(owner), /does not advertise HTTP MCP/);
  assert.ok(!(await calls()).some((call) => call.method === "session/new"));
});

test("shutdown interrupts a stalled initialize instead of waiting on the admission lock", { timeout: 5000 }, async (t) => {
  const { runtime, calls } = await setup(t, { requestTimeoutMs: 300000 }, { initializeWait: true });
  const rejected = assert.rejects(runtime.ensureSession(owner), /stopped/);
  await waitFor(async () => (await calls()).some((call) => call.method === "initialize"), "stalled initialize");
  await runtime.stop();
  await rejected;
});

test("shutdown interrupts a stalled control write and an in-flight doctor", { timeout: 5000 }, async (t) => {
  const { runtime, calls } = await setup(t, { requestTimeoutMs: 300000 }, { configWait: true });
  const handle = await runtime.ensureSession(owner);
  const rejected = assert.rejects(runtime.setConfigOption({ handle, key: "model", value: "fixture/deep" }), /stopped/);
  await waitFor(async () => (await calls()).some((call) => call.method === "session/set_config_option"), "stalled config write");
  await runtime.stop();
  await rejected;
  const probe = await setup(t, { requestTimeoutMs: 300000 }, { initializeWait: true });
  const pending = probe.runtime.doctor();
  await waitFor(async () => (await probe.calls()).some((call) => call.method === "initialize"), "stalled doctor");
  await probe.runtime.stop();
  assert.equal((await pending).ok, false);
});
