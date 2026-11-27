import { createHash, randomUUID } from "node:crypto";
import path from "node:path";
import { AcpConnection } from "./transport.mjs";
import { SessionStore, ownerKey } from "./store.mjs";
import { runtimeError } from "./events.mjs";
import { parseConfig } from "./config.mjs";

export class RoderSessions {
  constructor(config) {
    const { stateDir, ...options } = config;
    this.config = { ...parseConfig(options), stateDir };
    this.store = new SessionStore(stateDir);
    this.entries = new Map();
    this.locks = new Map();
    this.probes = new Set();
    this.opening = new Set();
    this.stopping = false;
    this.binding = createHash("sha256").update(JSON.stringify({
      command: this.config.command, args: this.config.args, mcpServers: this.config.mcpServers,
    })).digest("hex");
  }

  async locked(key, fn) {
    const previous = this.locks.get(key) ?? Promise.resolve();
    const next = previous.catch(() => {}).then(fn);
    this.locks.set(key, next);
    try { return await next; }
    finally { if (this.locks.get(key) === next) this.locks.delete(key); }
  }

  assertRunning() {
    if (this.stopping) throw runtimeError("ACP_BACKEND_UNAVAILABLE", "Roder runtime is stopped");
  }

  async open(record) {
    this.assertRunning();
    let entry;
    const conn = new AcpConnection({
      ...this.config, cwd: record.cwd,
      onNotification: (method, params) => this.onNotification(entry, method, params),
      onRequest: (method, params) => this.onRequest(entry, method, params),
    });
    this.opening.add(conn);
    entry = { record, conn, controls: {}, capabilities: {}, active: undefined };
    try {
      const initialized = await conn.start();
      if (initialized?.protocolVersion !== 1) throw new Error("Roder must support ACP v1");
      entry.capabilities = initialized.agentCapabilities ?? {};
      const method = record.sessionId ? "session/resume" : "session/new";
      if (!entry.capabilities.sessionCapabilities?.resume || !entry.capabilities.sessionCapabilities?.close) {
        throw new Error("Roder lacks session/resume or session/close; install a current Roder build");
      }
      if (this.config.mcpServers.some((server) => server.type === "http")
          && !entry.capabilities.mcpCapabilities?.http) {
        throw new Error("Roder does not advertise HTTP MCP support");
      }
      const result = await conn.request(method, {
        cwd: record.cwd, mcpServers: this.config.mcpServers,
        ...(record.sessionId ? { sessionId: record.sessionId } : {}),
      });
      if (!record.sessionId) {
        if (typeof result?.sessionId !== "string" || !result.sessionId) {
          throw new Error("Roder returned no session id");
        }
        record.sessionId = result.sessionId;
      }
      entry.controls = result ?? {};
      this.assertRunning();
      this.entries.set(ownerKey(record), entry);
      return entry;
    } catch (error) {
      await conn.stop();
      throw error;
    } finally {
      this.opening.delete(conn);
    }
  }

  handle(record) {
    return {
      sessionKey: record.sessionKey, ...(record.agentId ? { agentId: record.agentId } : {}),
      backend: "roder", runtimeSessionName: record.generation, cwd: record.cwd,
      backendSessionId: record.sessionId, agentSessionId: record.sessionId,
    };
  }

  async entryFor(handle) {
    this.assertRunning();
    const key = ownerKey(handle);
    const record = await this.store.read(key);
    if (!record || handle.backend !== "roder"
        || record.generation !== handle.runtimeSessionName
        || record.sessionId !== handle.backendSessionId) {
      throw runtimeError("ACP_SESSION_INIT_FAILED", "Stale or unknown Roder session handle");
    }
    if (record.binding !== this.binding) {
      throw runtimeError("ACP_SESSION_INIT_FAILED", "Roder launch or MCP configuration changed; start a fresh session");
    }
    let entry = this.entries.get(key);
    if (entry && !entry.conn.alive) {
      await entry.conn.stop();
      this.entries.delete(key);
      entry = undefined;
    }
    return entry ?? await this.open(record);
  }

  async ensureSession(input) {
    const key = ownerKey(input);
    return this.locked(key, async () => {
      this.assertRunning();
      if (input.agent !== "roder" || !["persistent", "oneshot"].includes(input.mode)) {
        throw runtimeError("ACP_SESSION_INIT_FAILED", "This backend only supports the roder agent in persistent or oneshot mode");
      }
      if (input.env !== undefined && (!input.env || typeof input.env !== "object"
          || Array.isArray(input.env) || Object.keys(input.env).length)) {
        throw runtimeError("ACP_INVALID_RUNTIME_OPTION", "Per-session environment overrides are unsupported; configure the gateway environment");
      }
      if (input.thinking && input.thinkingExplicit !== false) {
        throw runtimeError("ACP_INVALID_RUNTIME_OPTION", "Roder ACP does not advertise a thinking option");
      }
      if (input.resumeSessionId !== undefined
          && (typeof input.resumeSessionId !== "string" || !input.resumeSessionId)) {
        throw runtimeError("ACP_INVALID_RUNTIME_OPTION", "resumeSessionId must be nonempty");
      }
      if (input.cwd !== undefined && (typeof input.cwd !== "string"
          || !path.isAbsolute(input.cwd) || input.cwd.includes("\0"))) {
        throw runtimeError("ACP_INVALID_RUNTIME_OPTION", "Session cwd must be an absolute path without NUL bytes");
      }
      if (input.model !== undefined && (typeof input.model !== "string" || !input.model.trim())) {
        throw runtimeError("ACP_INVALID_RUNTIME_OPTION", "Model must be a nonblank advertised selection");
      }
      const cwd = path.resolve(input.cwd ?? this.config.cwd);
      let record = await this.store.read(key);
      if (record && (record.cwd !== cwd || record.mode !== input.mode
          || record.binding !== this.binding
          || (input.resumeSessionId && input.resumeSessionId !== record.sessionId))) {
        throw runtimeError("ACP_SESSION_INIT_FAILED", "Stored Roder session differs from requested workspace, mode, bindings or resume id");
      }
      if (!record && input.persistedHandle) {
        throw runtimeError("ACP_SESSION_INIT_FAILED", "Roder session mapping is missing; restore plugin state or explicitly start fresh");
      }
      if (!record && input.resumeSessionId) {
        throw runtimeError("ACP_SESSION_INIT_FAILED", "Cannot import an unmapped Roder session id; restore this owner's plugin state");
      }
      const created = !record;
      record ??= {
        version: 1, sessionKey: input.sessionKey,
        ...(input.agentId ? { agentId: input.agentId } : {}),
        sessionId: input.resumeSessionId ?? "", cwd, mode: input.mode,
        binding: this.binding, generation: randomUUID(),
      };
      let entry;
      try {
        entry = created ? await this.open(record) : await this.entryFor(this.handle(record));
        if (entry.active) throw runtimeError("ACP_SESSION_INIT_FAILED", "Roder session has an active turn");
        let appliedModel;
        if (input.model) {
          try { await this.applyOption(entry, "model", input.model); }
          catch (error) {
            // Only an unsupported inherited selection may be dropped. A broken
            // connection or failed config write is never a successful ensure.
            if (input.modelExplicit !== false || error.code !== "ACP_INVALID_RUNTIME_OPTION") throw error;
            appliedModel = { kind: "dropped" };
          }
        }
        this.assertRunning();
        await this.store.write(key, record);
        this.assertRunning();
        return {
          ...this.handle(record), ...(appliedModel ? { appliedModel } : {}),
          ...(input.thinking ? { appliedThinking: { kind: "dropped" } } : {}),
        };
      } catch (error) {
        if (created && entry) {
          this.entries.delete(key);
          await entry.conn.stop();
        }
        throw error;
      }
    });
  }

  async applyOption(entry, key, value) {
    const option = entry.controls.configOptions?.find((item) => item.id === key);
    const values = option?.options?.flatMap((item) => item.options ?? [item]) ?? [];
    if (!option || (values.length && !values.some((item) => item.value === value))) {
      throw runtimeError("ACP_INVALID_RUNTIME_OPTION", `Roder does not advertise the selected ${key} value`);
    }
    const result = await entry.conn.request("session/set_config_option", {
      sessionId: entry.record.sessionId, configId: key, value,
    });
    if (!Array.isArray(result?.configOptions)) throw new Error("Roder returned no accepted config options");
    entry.controls.configOptions = result.configOptions;
    return { configOptions: result.configOptions };
  }

  async prepareFreshSession(input) {
    return this.locked(ownerKey(input), () => {
      this.assertRunning();
      return this.discard(input);
    });
  }

  async discard(input) {
    const key = ownerKey(input);
    const entry = this.entries.get(key);
    if (entry?.active) await entry.active.cancel();
    if (entry) await entry.conn.stop();
    this.entries.delete(key);
    await this.store.delete(key);
  }

  async stop() {
    this.stopping = true;
    // Terminate setup/control requests before waiting on their admission locks.
    // Otherwise a stuck initialize or model write could delay gateway shutdown
    // for the full configured RPC timeout.
    await Promise.allSettled([
      ...[...this.probes, ...this.opening].map((conn) => conn.stop()),
      ...[...this.entries.values()].map(async (entry) => {
        if (entry.active) await entry.active.cancel();
        await entry.conn.stop();
      }),
      ...this.locks.values(),
    ]);
    this.entries.clear();
  }
}
