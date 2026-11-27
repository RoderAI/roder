import { ownerKey } from "./store.mjs";
import { deferred, EventQueue, promptBlocks, runtimeError, runtimeEvent } from "./events.mjs";
import { RoderSessions } from "./sessions.mjs";

export class RoderRuntime extends RoderSessions {
  ownerAwareSessions = 1;

  onNotification(entry, method, params) {
    if (!entry || method !== "session/update" || params?.sessionId !== entry.record.sessionId) return;
    const update = params.update;
    if (update?.sessionUpdate === "config_option_update") entry.controls.configOptions = update.configOptions;
    if (update?.sessionUpdate === "current_mode_update") {
      entry.controls.modes = { ...entry.controls.modes, currentModeId: update.currentModeId };
    }
    const event = runtimeEvent(update);
    if (event) entry.active?.emit(event);
  }

  async onRequest(entry, method, params) {
    if (method !== "session/request_permission") {
      throw Object.assign(new Error("Unsupported ACP client request"), { rpcCode: -32601 });
    }
    if (!entry?.active || entry.active.isCancelled() || params?.sessionId !== entry.record.sessionId) {
      return { outcome: { outcome: "cancelled" } };
    }
    const read = ["read", "search"].includes(params.toolCall?.kind);
    const allow = this.config.permissionMode === "approve-all"
      || (this.config.permissionMode === "approve-reads" && read);
    const option = params.options?.find((item) => item.kind === (allow ? "allow_once" : "reject_once"))
      ?? (!allow ? params.options?.find((item) => item.kind === "reject_always") : undefined);
    return option ? { outcome: { outcome: "selected", optionId: option.optionId } }
      : { outcome: { outcome: "cancelled" } };
  }

  startTurn(input) {
    const queue = new EventQueue();
    const terminal = deferred();
    const started = deferred();
    // Hosts that only use runTurn need not consume promptStarted.
    started.promise.catch(() => {});
    let entry, cancelled = false, finished = false, cancelPromise;
    const cancel = () => {
      cancelled = true;
      if (!entry || finished) return Promise.resolve();
      cancelPromise ??= (async () => {
        await entry.conn.notify("session/cancel", { sessionId: entry.record.sessionId }).catch(() => {});
        let timer;
        try {
          await Promise.race([
            terminal.promise,
            new Promise((resolve) => {
              timer = setTimeout(async () => { await entry.conn.stop(); resolve(); }, this.config.cancelTimeoutMs);
            }),
          ]);
        } finally { clearTimeout(timer); }
      })();
      return cancelPromise;
    };
    const active = {
      cancel,
      isCancelled: () => cancelled,
      emit: (event) => {
        try { queue.push(event); }
        catch {
          // Do not let slow consumers grow the gateway without bound.
          void entry.conn.stop();
        }
      },
    };
    const abort = () => { void cancel(); };
    input.signal?.addEventListener("abort", abort, { once: true });
    if (input.signal?.aborted) cancelled = true;
    void (async () => {
      let result;
      try {
        if (input.mode !== "prompt") {
          throw runtimeError("ACP_BACKEND_UNSUPPORTED_CONTROL", "Roder ACP does not support steer; cancel and send a new prompt");
        }
        const prompt = promptBlocks(input.text, input.attachments);
        if (cancelled) {
          result = { status: "cancelled", stopReason: "cancelled" };
          return;
        }
        await this.locked(ownerKey(input.handle), async () => {
          entry = await this.entryFor(input.handle);
          if (entry.active) throw runtimeError("ACP_TURN_FAILED", "Roder session already has an active turn");
          if (prompt.some((block) => block.type === "image") && !entry.capabilities.promptCapabilities?.image) {
            throw runtimeError("ACP_INVALID_RUNTIME_OPTION", "Roder does not advertise image prompts");
          }
          if (!cancelled) entry.active = active;
        });
        if (cancelled) {
          result = { status: "cancelled", stopReason: "cancelled" };
          return;
        }
        const response = entry.conn.request("session/prompt", {
          sessionId: entry.record.sessionId, prompt,
        }, { timeoutMs: 0 });
        started.resolve();
        const done = await response;
        if (typeof done?.stopReason !== "string") throw new Error("Roder returned no prompt stop reason");
        result = { status: cancelled || done.stopReason === "cancelled" ? "cancelled" : "completed", stopReason: done.stopReason };
      } catch (error) {
        started.reject(error);
        result = cancelled ? { status: "cancelled", stopReason: "cancelled" }
          : { status: "failed", error: this.turnError(error) };
      } finally {
        finished = true;
        input.signal?.removeEventListener("abort", abort);
        if (entry?.active === active) entry.active = undefined;
        started.resolve();
        queue.end();
        terminal.resolve(result);
      }
    })();
    return {
      requestId: input.requestId, promptStarted: started.promise,
      events: queue, result: terminal.promise, cancel,
      closeStream: async () => { queue.end(true); await cancel(); },
    };
  }

  turnError(error) {
    let message = error instanceof Error ? error.message : "Roder turn failed";
    for (const server of this.config.mcpServers) {
      for (const { value } of [...(server.env ?? []), ...(server.headers ?? [])]) {
        if (value) message = message.split(value).join("[redacted]");
      }
    }
    return {
      code: typeof error.code === "string" && error.code.startsWith("ACP_") ? error.code : "ACP_TURN_FAILED",
      ...(error.rpcCode === -32000 ? { detailCode: "AUTH_REQUIRED" } : {}),
      message,
    };
  }

  async *runTurn(input) {
    const turn = this.startTurn(input);
    let drained = false;
    try {
      yield* turn.events;
      const result = await turn.result;
      drained = true;
      if (result.status === "failed") yield { type: "error", ...result.error };
      else yield { type: "done", ...result };
    } finally {
      if (!drained) await turn.closeStream();
    }
  }

  async getCapabilities({ handle } = {}) {
    if (!handle) return { controls: ["session/set_mode", "session/set_config_option", "session/status"], configOptionKeys: ["model"] };
    return this.locked(ownerKey(handle), async () => {
      const entry = await this.entryFor(handle);
      return {
        controls: [
          ...(entry.controls.modes ? ["session/set_mode"] : []),
          ...(entry.controls.configOptions?.length ? ["session/set_config_option"] : []),
          "session/status",
        ],
        configOptionKeys: (entry.controls.configOptions ?? []).map((item) => item.id),
      };
    });
  }

  async getStatus({ handle, signal }) {
    if (signal?.aborted) throw signal.reason ?? new Error("Status request aborted");
    return this.locked(ownerKey(handle), async () => {
      const entry = await this.entryFor(handle);
      return {
        summary: entry.active ? "Roder is running" : "Roder is idle",
        backendSessionId: entry.record.sessionId, agentSessionId: entry.record.sessionId,
        details: { cwd: entry.record.cwd, mode: entry.controls.modes?.currentModeId, configOptions: entry.controls.configOptions ?? [] },
      };
    });
  }

  async setMode({ handle, mode }) {
    return this.locked(ownerKey(handle), async () => {
      const entry = await this.entryFor(handle);
      if (entry.active) throw runtimeError("ACP_INVALID_RUNTIME_OPTION", "Cannot change mode during a turn");
      if (!entry.controls.modes?.availableModes?.some((item) => item.id === mode)) {
        throw runtimeError("ACP_INVALID_RUNTIME_OPTION", "Roder does not advertise the selected mode");
      }
      await entry.conn.request("session/set_mode", { sessionId: entry.record.sessionId, modeId: mode });
      entry.controls.modes.currentModeId = mode;
    });
  }

  async setConfigOption({ handle, key, value }) {
    return this.locked(ownerKey(handle), async () => {
      const entry = await this.entryFor(handle);
      if (entry.active) throw runtimeError("ACP_INVALID_RUNTIME_OPTION", "Cannot change configuration during a turn");
      return this.applyOption(entry, key, value);
    });
  }

  async cancel({ handle }) {
    // Cancellation must not wait behind a prompt or session-close request.
    const entry = this.entries.get(ownerKey(handle));
    if (!entry) {
      const record = await this.store.read(ownerKey(handle));
      if (record?.generation === handle.runtimeSessionName && handle.backend === "roder"
          && record.sessionId === handle.backendSessionId) return;
    }
    if (!entry || entry.record.generation !== handle.runtimeSessionName
        || handle.backend !== "roder" || entry.record.sessionId !== handle.backendSessionId) {
      throw runtimeError("ACP_SESSION_INIT_FAILED", "Stale or unknown Roder session handle");
    }
    await entry.active?.cancel();
  }

  async close({ handle, discardPersistentState = false }) {
    return this.locked(ownerKey(handle), async () => {
      const key = ownerKey(handle);
      const record = await this.store.read(key);
      if (!record || record.generation !== handle.runtimeSessionName || handle.backend !== "roder"
          || record.sessionId !== handle.backendSessionId) {
        throw runtimeError("ACP_SESSION_INIT_FAILED", "Stale or unknown Roder session handle");
      }
      const entry = this.entries.get(key);
      if (entry?.active) await entry.active.cancel();
      try {
        if (entry?.conn.alive && entry.capabilities.sessionCapabilities?.close) {
          await entry.conn.request("session/close", { sessionId: record.sessionId });
        }
      } finally {
        if (entry) await entry.conn.stop();
        this.entries.delete(key);
        if (discardPersistentState || record.mode === "oneshot") await this.store.delete(key);
      }
    });
  }

  async doctor() {
    const { AcpConnection } = await import("./transport.mjs");
    this.assertRunning();
    const conn = new AcpConnection(this.config);
    this.probes.add(conn);
    try {
      const result = await conn.start();
      const capabilities = result.agentCapabilities;
      const ok = result.protocolVersion === 1 && !!capabilities?.sessionCapabilities?.resume
        && !!capabilities?.sessionCapabilities?.close;
      return {
        ok, code: ok ? undefined : "ACP_BACKEND_UNAVAILABLE",
        message: ok ? "Roder ACP v1 is ready (transport and lifecycle only)" : "Install a Roder build with ACP resume/close support",
        details: ["Provider authentication and real-model/tool access require a separate prompt smoke test."],
      };
    } catch (error) {
      return { ok: false, code: "ACP_BACKEND_UNAVAILABLE", message: this.turnError(error).message };
    } finally {
      await conn.stop();
      this.probes.delete(conn);
    }
  }
}
