import { spawn } from "node:child_process";

const DEFAULT_MAX_MESSAGE_BYTES = 16 * 1024 * 1024;
const STDERR_BYTES = 64 * 1024;
const SHUTDOWN_MS = 500;

function abortError() {
  const error = new Error("ACP request aborted");
  error.name = "AbortError";
  return error;
}

/** A newline-delimited JSON-RPC connection to a single ACP subprocess. */
export class AcpConnection {
  constructor({
    command,
    args = [],
    cwd,
    requestTimeoutMs = 30_000,
    maxMessageBytes = DEFAULT_MAX_MESSAGE_BYTES,
    onNotification,
    onRequest,
  }) {
    if (typeof command !== "string" || !command) {
      throw new TypeError("ACP command must be a nonempty string");
    }
    if (!Array.isArray(args) || args.some((arg) => typeof arg !== "string")) {
      throw new TypeError("ACP args must be an array of strings");
    }
    if (!Number.isFinite(requestTimeoutMs) || requestTimeoutMs < 0) {
      throw new TypeError("ACP requestTimeoutMs must be nonnegative");
    }
    if (!Number.isSafeInteger(maxMessageBytes) || maxMessageBytes < 1) {
      throw new TypeError("ACP maxMessageBytes must be a positive integer");
    }
    this.config = { command, args, cwd, requestTimeoutMs, maxMessageBytes };
    this.onNotification = onNotification;
    this.onRequest = onRequest;
    this.child = undefined;
    this.pending = new Map();
    this.nextId = 1;
    this.fragments = [];
    this.fragmentBytes = 0;
    this.stderrBuffer = Buffer.alloc(0);
    this.failure = undefined;
    this.stopping = false;
  }

  get alive() {
    return Boolean(this.child && !this.failure && !this.stopping &&
      this.child.exitCode === null && this.child.signalCode === null);
  }

  get stderrTail() {
    return this.stderrBuffer.toString("utf8");
  }

  start() {
    if (!this.startPromise) this.startPromise = this.startChild();
    return this.startPromise;
  }

  async startChild() {
    if (this.stopping) throw new Error("ACP connection is stopped");
    const { command, args, cwd } = this.config;
    const child = spawn(command, args, {
      cwd,
      shell: false,
      env: process.env,
      stdio: ["pipe", "pipe", "pipe"],
      // POSIX descendants inherit this private group. Windows only supports
      // terminating the directly spawned child with the built-in Node API.
      detached: process.platform !== "win32",
    });
    this.child = child;
    this.ownedGroup = process.platform !== "win32" ? child.pid : undefined;
    this.closed = new Promise((resolve) => {
      child.once("close", () => {
        this.childClosed = true;
        this.cleanReaders();
        resolve();
      });
    });
    child.on("error", (error) => this.fail(error));
    child.once("exit", (code, signal) => {
      this.fail(new Error(`ACP process exited (${signal ?? code})`), !this.stopping);
    });
    child.stdout.on("data", (chunk) => this.read(chunk));
    child.stdout.on("error", (error) => this.fail(error));
    child.stderr.on("data", (chunk) => {
      this.stderrBuffer = Buffer.concat([this.stderrBuffer, chunk]).subarray(-STDERR_BYTES);
    });
    child.stderr.on("error", (error) => this.fail(error));
    child.stdin.on("error", (error) => this.fail(error));
    try {
      return await this.request("initialize", {
        protocolVersion: 1,
        clientCapabilities: {},
      });
    } catch (error) {
      await this.stop();
      throw error;
    }
  }

  request(method, params, { timeoutMs = this.config.requestTimeoutMs, signal } = {}) {
    if (typeof method !== "string" || !method) {
      return Promise.reject(new TypeError("ACP method must be a nonempty string"));
    }
    if (!Number.isFinite(timeoutMs) || timeoutMs < 0) {
      return Promise.reject(new TypeError("ACP timeoutMs must be nonnegative"));
    }
    if (signal?.aborted) return Promise.reject(abortError());
    if (!this.alive) return Promise.reject(this.failure ?? new Error("ACP connection is not running"));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      let timer;
      const finish = (error, result) => {
        if (!this.pending.delete(id)) return;
        clearTimeout(timer);
        signal?.removeEventListener("abort", onAbort);
        if (error) reject(error);
        else resolve(result);
      };
      const onAbort = () => finish(abortError());
      this.pending.set(id, { finish });
      signal?.addEventListener("abort", onAbort, { once: true });
      if (timeoutMs > 0) {
        timer = setTimeout(() => {
          const error = new Error(`ACP request timed out: ${method}`);
          error.name = "TimeoutError";
          finish(error);
        }, timeoutMs);
      }
      this.write({ jsonrpc: "2.0", id, method, params }).catch((error) => finish(error));
    });
  }

  notify(method, params) {
    if (typeof method !== "string" || !method) {
      return Promise.reject(new TypeError("ACP method must be a nonempty string"));
    }
    return this.write({ jsonrpc: "2.0", method, params });
  }

  async write(message) {
    if (!this.alive) throw this.failure ?? new Error("ACP connection is not running");
    const data = `${JSON.stringify(message)}\n`;
    if (Buffer.byteLength(data) - 1 > this.config.maxMessageBytes) {
      throw new Error("ACP outgoing message exceeds maxMessageBytes");
    }
    await new Promise((resolve, reject) => {
      this.child.stdin.write(data, (error) => error ? reject(error) : resolve());
    });
  }

  read(chunk) {
    if (this.failure || this.stopping) return;
    let offset = 0;
    while (offset < chunk.length) {
      const newline = chunk.indexOf(10, offset);
      const end = newline === -1 ? chunk.length : newline;
      const part = chunk.subarray(offset, end);
      if (this.fragmentBytes + part.length > this.config.maxMessageBytes) {
        this.fail(new Error("ACP message exceeds maxMessageBytes"));
        return;
      }
      this.fragments.push(part);
      this.fragmentBytes += part.length;
      if (newline === -1) return;
      const bytes = Buffer.concat(this.fragments, this.fragmentBytes);
      this.fragments = [];
      this.fragmentBytes = 0;
      try {
        const line = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
        this.receive(JSON.parse(line));
      } catch {
        // JSON.parse diagnostics can echo the offending payload, including
        // credentials from a misconfigured executable's stdout.
        this.fail(new Error("Invalid ACP JSON-RPC message"));
        return;
      }
      offset = newline + 1;
    }
  }

  receive(message) {
    if (!message || typeof message !== "object" || Array.isArray(message) ||
        message.jsonrpc !== "2.0") {
      throw new Error("expected a JSON-RPC 2.0 object");
    }
    if ("method" in message) {
      if (typeof message.method !== "string") throw new Error("invalid method");
      if ("id" in message) {
        this.validateId(message.id);
        void this.answer(message);
      } else {
        // A consumer callback must not create unhandled promise rejections.
        Promise.resolve().then(() => this.onNotification?.(message.method, message.params))
          .catch(() => {});
      }
      return;
    }
    this.validateId(message.id);
    if (("result" in message) === ("error" in message)) {
      throw new Error("expected exactly one of result or error");
    }
    let error;
    if ("error" in message) {
      if (!message.error || typeof message.error.code !== "number" ||
          typeof message.error.message !== "string") {
        throw new Error("invalid JSON-RPC error");
      }
      error = new Error(message.error.message);
      error.rpcCode = message.error.code;
      if ("data" in message.error) error.data = message.error.data;
    }
    this.pending.get(message.id)?.finish(error, message.result);
  }

  validateId(id) {
    if (id !== null && typeof id !== "string" &&
        !(typeof id === "number" && Number.isFinite(id))) {
      throw new Error("invalid JSON-RPC id");
    }
  }

  async answer(message) {
    let response;
    try {
      if (!this.onRequest) {
        const error = new Error(`Unsupported ACP request: ${message.method}`);
        error.rpcCode = -32601;
        throw error;
      }
      const result = await this.onRequest(message.method, message.params);
      response = { result: result ?? null };
    } catch (error) {
      response = {
        error: {
          code: Number.isInteger(error?.rpcCode) ? error.rpcCode : -32603,
          message: error instanceof Error ? error.message : String(error),
        },
      };
    }
    try {
      await this.write({ jsonrpc: "2.0", id: message.id, ...response });
    } catch (error) {
      if (this.alive) this.fail(error);
    }
  }

  fail(error, terminate = true) {
    if (this.failure) return;
    this.failure = error;
    for (const { finish } of this.pending.values()) finish(error);
    this.fragments = [];
    this.fragmentBytes = 0;
    if (terminate) this.signalOwned("SIGKILL");
  }

  groupAlive() {
    if (!this.ownedGroup) return false;
    try {
      process.kill(-this.ownedGroup, 0);
      return true;
    } catch (error) {
      if (error.code !== "ESRCH") return true;
      this.ownedGroup = undefined;
      return false;
    }
  }

  signalOwned(signal) {
    if (this.ownedGroup) {
      try { process.kill(-this.ownedGroup, signal); }
      catch (error) { if (error.code !== "ESRCH") throw error; }
      // Never reuse a retained process-group id after final termination.
      if (signal === "SIGKILL") this.ownedGroup = undefined;
    } else if (this.child?.exitCode === null && this.child?.signalCode === null) {
      this.child.kill(signal);
    }
  }

  async waitForShutdown() {
    const deadline = Date.now() + SHUTDOWN_MS;
    while (!this.childClosed || this.groupAlive()) {
      const remaining = deadline - Date.now();
      if (remaining <= 0) return false;
      await new Promise((resolve) => setTimeout(resolve, Math.min(25, remaining)));
    }
    return true;
  }

  cleanReaders() {
    for (const stream of [this.child?.stdout, this.child?.stderr, this.child?.stdin]) {
      stream?.removeAllListeners("data");
      stream?.destroy();
    }
    this.fragments = [];
    this.fragmentBytes = 0;
  }

  stop() {
    if (!this.stopPromise) this.stopPromise = this.stopChild();
    return this.stopPromise;
  }

  async stopChild() {
    this.stopping = true;
    const error = new Error("ACP connection stopped");
    for (const { finish } of this.pending.values()) finish(error);
    if (!this.child) return;
    // EOF lets the ACP agent drain native sessions and MCP clients first.
    this.child.stdin.end();
    if (!await this.waitForShutdown()) {
      this.signalOwned("SIGTERM");
      if (!await this.waitForShutdown()) {
        this.signalOwned("SIGKILL");
        await this.waitForShutdown();
      }
    }
    this.cleanReaders();
  }
}
