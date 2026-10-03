import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createInterface, type Interface } from "node:readline";
import type {AppServerMethod, JsonRpcRequest, JsonRpcResponse} from "./types.generated.js";
import type {RoderTransport, RequestOptions, JsonRpcNotification} from "./transports.js";
import {RoderTransportError} from "./errors.js";
import {NotificationHub, throwIfAborted, isNotification, type PendingResponse} from "./transport-support.js";
export interface LocalProcessTransportOptions {
  command?: string;
  args?: string[];
  cwd?: string;
  env?: NodeJS.ProcessEnv;
  /**
   * When false, the spawned app-server receives exactly `env` instead of inheriting the host
   * process environment merged with `env`. Hosts holding secrets the server must not see (and
   * that could surface through its stderr tail) pass an explicit allowlist this way. Defaults
   * to true.
   */
  inheritEnv?: boolean;
}

/** Number of recent stderr lines retained for error reporting. */
const STDERR_TAIL_LINES = 50;

export class LocalProcessTransport implements RoderTransport {
  private readonly process: ChildProcessWithoutNullStreams;
  private readonly lines: Interface;
  private readonly stderrLines: Interface;
  private readonly stderrTail: string[] = [];
  private readonly pending = new Map<string, PendingResponse>();
  private readonly notificationHub = new NotificationHub();
  private closed = false;
  private readonly lifetime = new AbortController();
  readonly closedSignal = this.lifetime.signal;

  constructor(options: LocalProcessTransportOptions = {}) {
    const command = options.command ?? "roder";
    const args = options.args ?? ["app-server", "--listen", "stdio://"];
    this.process = spawn(command, args, {
      cwd: options.cwd,
      env: options.inheritEnv === false ? { ...options.env } : { ...process.env, ...options.env },
      stdio: "pipe",
    });
    this.lines = createInterface({ input: this.process.stdout });
    this.lines.on("line", (line: string) => this.handleLine(line));
    /**
     * stderr must be drained continuously: a chatty server fills the pipe
     * buffer and blocks, deadlocking the turn. Keep a bounded tail for error
     * reporting instead of logging.
     */
    this.stderrLines = createInterface({ input: this.process.stderr });
    this.stderrLines.on("line", (line: string) => {
      this.stderrTail.push(line);
      if (this.stderrTail.length > STDERR_TAIL_LINES) {
        this.stderrTail.shift();
      }
    });
    /**
     * "close" (not "exit") so the stdio pipes are fully drained and the
     * stderr tail is complete before pending requests are rejected.
     */
    this.process.once("close", (code: number | null, signal: NodeJS.Signals | null) => {
      this.rejectAll(
        new RoderTransportError(this.withStderrTail(`app-server exited code=${code} signal=${signal}`)),
      );
      this.notificationHub.close();
    });
    this.process.once("error", (error: Error) => {
      this.rejectAll(
        new RoderTransportError(this.withStderrTail("failed to start app-server"), { cause: error }),
      );
      this.notificationHub.close();
    });
  }

  async request<M extends AppServerMethod, P = unknown, R = unknown>(
    request: JsonRpcRequest<M, P>,
    options: RequestOptions = {},
  ): Promise<JsonRpcResponse<R>> {
    throwIfAborted(options.signal);
    if (this.closed) {
      return Promise.reject(new RoderTransportError("transport is closed"));
    }
    const id = request.id;
    if (id === undefined || id === null) {
      return Promise.reject(new RoderTransportError("requests require a non-null id"));
    }
    const serialized = `${JSON.stringify(request)}\n`;
    const key = JSON.stringify(id);
    if (this.pending.has(key)) return Promise.reject(new RoderTransportError("Request id is already pending"));
    const promise = new Promise<JsonRpcResponse<R>>((resolve, reject) => {
      const abort = () => {
        this.pending.delete(key);
        reject(new DOMException("Request aborted", "AbortError"));
      };
      if (options.signal) {
        options.signal.addEventListener("abort", abort, { once: true });
      }
      this.pending.set(key, {
        resolve: (response) => resolve(response as JsonRpcResponse<R>),
        reject,
        cleanup: () => options.signal?.removeEventListener("abort", abort),
      });
    });
    this.process.stdin.write(serialized);
    return promise;
  }

  notifications(): AsyncIterable<JsonRpcNotification> {
    return this.notificationHub.subscribe();
  }

  async close(): Promise<void> {
    this.closed = true;
    this.lifetime.abort();
    this.lines.close();
    this.stderrLines.close();
    this.notificationHub.close();
    this.process.stdin.end();
    this.process.kill();
    this.rejectAll(new RoderTransportError("transport is closed"));
  }

  private withStderrTail(message: string): string {
    if (this.stderrTail.length === 0) {
      return message;
    }
    return `${message}\nrecent stderr (last ${this.stderrTail.length} lines):\n${this.stderrTail.join("\n")}`;
  }

  private handleLine(line: string): void {
    let message: JsonRpcResponse | JsonRpcNotification;
    try {
      const parsed: unknown = JSON.parse(line);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("invalid JSON-RPC frame");
      message = parsed as JsonRpcResponse | JsonRpcNotification;
    } catch (error) {
      this.rejectAll(new RoderTransportError("Invalid app-server JSON-RPC frame", {cause: error}));
      void this.close();
      return;
    }
    if ("id" in message) {
      const key = JSON.stringify(message.id);
      const pending = this.pending.get(key);
      if (pending) {
        this.pending.delete(key);
        pending.cleanup();
        pending.resolve(message);
      }
      return;
    }
    if (isNotification(message)) {
      this.notificationHub.push(message);
    }
  }

  private rejectAll(error: Error): void {
    this.closed = true;
    this.lifetime.abort();
    for (const pending of this.pending.values()) {
      pending.cleanup();
      pending.reject(error);
    }
    this.pending.clear();
  }
}
