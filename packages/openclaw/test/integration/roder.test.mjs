import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { parseConfig } from "../../src/config.mjs";
import { RoderRuntime } from "../../src/runtime.mjs";

const binary = process.env.RODER_TEST_BINARY;

// Deliberately outside test/*.test.mjs: this needs an explicitly supplied real
// Roder build, but never provider credentials or a network-backed model.
test("real Roder ACP lifecycle with the offline mock provider", {
  skip: !binary && "set RODER_TEST_BINARY to an ACP-enabled Roder binary",
  timeout: 120_000,
}, async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "roder-openclaw-integration-"));
  const configDir = path.join(root, "config");
  const cwd = path.join(root, "workspace");
  await mkdir(configDir);
  await mkdir(cwd);
  await writeFile(path.join(configDir, "config.toml"),
    'provider = "mock"\nmodel = "mock"\n', { mode: 0o600 });
  const environment = {
    RODER_CONFIG_DIR: configDir,
    RODER_DATA_DIR: path.join(root, "data"),
    // Explicitly override a developer's inherited live-provider selection.
    RODER_PROVIDER: "mock",
    RODER_MODEL: "mock",
  };
  const previous = new Map(Object.keys(environment).map((key) => [key, process.env[key]]));
  Object.assign(process.env, environment);
  const runtimes = [];
  t.after(async () => {
    try {
      await Promise.all(runtimes.map((runtime) => runtime.stop()));
    } finally {
      for (const [key, value] of previous) {
        if (value === undefined) delete process.env[key];
        else process.env[key] = value;
      }
      await rm(root, { recursive: true, force: true });
    }
  });

  const config = {
    ...parseConfig({ command: path.resolve(binary), args: ["acp"], cwd }),
    stateDir: path.join(root, "plugin"),
  };
  const createRuntime = () => {
    const runtime = new RoderRuntime(config);
    runtimes.push(runtime);
    return runtime;
  };
  const input = {
    sessionKey: "offline-integration",
    agentId: "integration-agent",
    agent: "roder",
    mode: "persistent",
    cwd,
  };
  let runtime = createRuntime();
  let handle;
  let request = 0;
  async function prompt(text) {
    const events = [];
    for await (const event of runtime.runTurn({
      handle, requestId: `integration-${++request}`, mode: "prompt", text,
    })) {
      events.push(event);
    }
    assert.equal(events.at(-1)?.type, "done", JSON.stringify(events));
    assert.equal(events.at(-1)?.status, "completed", JSON.stringify(events));
    const output = events.filter((event) =>
      event.type === "text_delta" && event.stream === "output");
    assert.ok(output.length > 0, "real process must stream output events");
    assert.equal(output.map((event) => event.text).join(""), "hello from roder");
    return events;
  }

  await t.test("doctor initializes the real ACP subprocess", async () => {
    const result = await runtime.doctor();
    assert.equal(result.ok, true, JSON.stringify(result));
  });

  await t.test("creates an owner-bound persistent native session", async () => {
    handle = await runtime.ensureSession(input);
    assert.equal(handle.backend, "roder");
    assert.equal(handle.cwd, cwd);
    assert.equal(handle.agentId, input.agentId);
    assert.ok(handle.backendSessionId);
    assert.equal(handle.agentSessionId, handle.backendSessionId);
    assert.deepEqual(await runtime.ensureSession(input), handle);
  });

  await t.test("advertised controls accept exact mock model and current mode values", async () => {
    const capabilities = await runtime.getCapabilities({ handle });
    assert.ok(capabilities.controls.includes("session/set_mode"));
    assert.ok(capabilities.controls.includes("session/set_config_option"));
    assert.ok(capabilities.controls.includes("session/status"));
    assert.ok(capabilities.configOptionKeys.includes("model"));
    const status = await runtime.getStatus({ handle });
    assert.equal(status.backendSessionId, handle.backendSessionId);
    const model = status.details.configOptions.find((option) => option.id === "model");
    assert.ok(model);
    const options = model.options.flatMap((option) => option.options ?? [option]);
    const selected = options.find((option) => option.value === "mock/mock");
    assert.ok(selected, "mock/mock must actually be advertised by this binary");
    await runtime.setConfigOption({ handle, key: "model", value: selected.value });
    const after = await runtime.getStatus({ handle });
    assert.equal(after.details.configOptions.find((option) => option.id === "model").currentValue,
      selected.value);
    await runtime.setMode({ handle, mode: after.details.mode });
    await assert.rejects(runtime.setConfigOption({
      handle, key: "model", value: "not-advertised/not-advertised",
    }), { code: "ACP_INVALID_RUNTIME_OPTION" });
    await assert.rejects(runtime.setMode({ handle, mode: "not-advertised" }),
      { code: "ACP_INVALID_RUNTIME_OPTION" });
  });

  await t.test("streams real output and permits an immediate follow-up", async () => {
    const events = await prompt("Say a short greeting.");
    const thoughts = events.filter((event) => event.stream === "thought");
    // FakeInferenceEngine is text_only and emits MessageDelta, not reasoning.
    assert.equal(thoughts.length, 0);
    await prompt("Say another short greeting.");
  });

  await t.test("cancels an active prompt and allows another turn", async () => {
    const turn = runtime.startTurn({
      handle, requestId: `integration-${++request}`, mode: "prompt",
      text: "Say a short greeting.",
    });
    const consume = (async () => {
      const events = [];
      for await (const event of turn.events) events.push(event);
      return events;
    })();
    await turn.promptStarted;
    await turn.cancel();
    assert.equal((await turn.result).status, "cancelled");
    await consume;
    await prompt("Say a short greeting after cancellation.");
  });

  await t.test("close/reopen resumes the same native session", async () => {
    const previousHandle = handle;
    await runtime.close({ handle });
    handle = await runtime.ensureSession({ ...input, persistedHandle: previousHandle });
    assert.equal(handle.backendSessionId, previousHandle.backendSessionId);
    assert.equal(handle.runtimeSessionName, previousHandle.runtimeSessionName);
    await prompt("Say a greeting after reopening.");
  });

  await t.test("a cold runtime resumes the persisted native session", async () => {
    const previousHandle = handle;
    await runtime.stop();
    runtime = createRuntime();
    handle = await runtime.ensureSession({ ...input, persistedHandle: previousHandle });
    assert.equal(handle.backendSessionId, previousHandle.backendSessionId);
    assert.equal(handle.runtimeSessionName, previousHandle.runtimeSessionName);
    await prompt("Say a greeting after restarting.");
  });

  await t.test("fresh reset rotates native identity and rejects stale handles", async () => {
    const stale = handle;
    await runtime.prepareFreshSession(input);
    handle = await runtime.ensureSession(input);
    assert.notEqual(handle.backendSessionId, stale.backendSessionId);
    assert.notEqual(handle.runtimeSessionName, stale.runtimeSessionName);
    await assert.rejects(runtime.getStatus({ handle: stale }),
      { code: "ACP_SESSION_INIT_FAILED" });
    await prompt("Say a greeting in the fresh session.");
    await runtime.close({ handle, discardPersistentState: true });
    await assert.rejects(runtime.getStatus({ handle }),
      { code: "ACP_SESSION_INIT_FAILED" });
  });
});
