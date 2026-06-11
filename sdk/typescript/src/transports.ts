import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createInterface, type Interface } from "node:readline";
import type { AppServerMethod, JsonRpcId, JsonRpcRequest, JsonRpcResponse } from "./types.generated.js";
import { RoderTransportError } from "./errors.js";

export interface JsonRpcNotification<P = unknown> {
  jsonrpc: "2.0";
  method: string;
  params?: P;
}

export interface RoderTransport {
  request<M extends AppServerMethod, P = unknown, R = unknown>(
    request: JsonRpcRequest<M, P>,
    options?: RequestOptions,
  ): Promise<JsonRpcResponse<R>>;
  notifications(): AsyncIterable<JsonRpcNotification>;
  close(): Promise<void> | void;
}

export interface RequestOptions {
  signal?: AbortSignal;
}

export type InMemoryHandler = (
  request: JsonRpcRequest,
) => JsonRpcResponse | Promise<JsonRpcResponse>;

export class InMemoryTransport implements RoderTransport {
  private readonly notificationHub = new NotificationHub();
  private closed = false;

  constructor(private readonly handler: InMemoryHandler) {}

  async request<M extends AppServerMethod, P = unknown, R = unknown>(
    request: JsonRpcRequest<M, P>,
    options: RequestOptions = {},
  ): Promise<JsonRpcResponse<R>> {
    throwIfAborted(options.signal);
    if (this.closed) {
      throw new RoderTransportError("transport is closed");
    }
    const response = await abortable(Promise.resolve(this.handler(request)), options.signal);
    return response as JsonRpcResponse<R>;
  }

  emit(notification: JsonRpcNotification): void {
    this.notificationHub.push(notification);
  }

  notifications(): AsyncIterable<JsonRpcNotification> {
    return this.notificationHub.subscribe();
  }

  close(): void {
    this.closed = true;
    this.notificationHub.close();
  }
}

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
  startupTimeoutMs?: number;
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

  request<M extends AppServerMethod, P = unknown, R = unknown>(
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
    const key = String(id);
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
    this.process.stdin.write(`${JSON.stringify(request)}\n`);
    return promise;
  }

  notifications(): AsyncIterable<JsonRpcNotification> {
    return this.notificationHub.subscribe();
  }

  async close(): Promise<void> {
    this.closed = true;
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
    const message = JSON.parse(line) as JsonRpcResponse | JsonRpcNotification;
    if ("id" in message) {
      const key = String(message.id);
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
    for (const pending of this.pending.values()) {
      pending.cleanup();
      pending.reject(error);
    }
    this.pending.clear();
  }
}

export interface WebSocketTransportOptions {
  url: string;
  token?: string;
  /** Extra handshake headers (e.g. externally supplied auth headers). */
  headers?: Record<string, string>;
  protocols?: string[];
  webSocketFactory?: WebSocketFactory;
}

export type WebSocketFactory = (
  url: string,
  protocols: string[],
  options: { headers?: Record<string, string> },
) => WebSocketLike;

export interface WebSocketLike {
  readyState: number;
  send(data: string): void;
  close(): void;
  addEventListener(type: "open" | "message" | "error" | "close", listener: (event: any) => void): void;
}

export class WebSocketTransport implements RoderTransport {
  private readonly socket: WebSocketLike;
  private readonly opened: Promise<void>;
  private readonly pending = new Map<string, PendingResponse>();
  private readonly notificationHub = new NotificationHub();

  constructor(options: WebSocketTransportOptions) {
    const protocols = options.protocols ?? [];
    const headers: Record<string, string> | undefined =
      options.token || options.headers
        ? {
            ...(options.token ? { Authorization: `Bearer ${options.token}` } : {}),
            ...options.headers,
          }
        : undefined;
    const factory = options.webSocketFactory ?? defaultWebSocketFactory;
    this.socket = factory(options.url, protocols, { headers });
    this.opened = new Promise((resolve, reject) => {
      this.socket.addEventListener("open", () => resolve());
      this.socket.addEventListener("error", (event) =>
        reject(new RoderTransportError("websocket connection failed", { cause: event })),
      );
    });
    this.socket.addEventListener("message", (event) => this.handleMessage(String(event.data)));
    this.socket.addEventListener("close", () => {
      this.rejectAll(new RoderTransportError("websocket closed"));
      this.notificationHub.close();
    });
  }

  async request<M extends AppServerMethod, P = unknown, R = unknown>(
    request: JsonRpcRequest<M, P>,
    options: RequestOptions = {},
  ): Promise<JsonRpcResponse<R>> {
    throwIfAborted(options.signal);
    await abortable(this.opened, options.signal);
    const id = request.id;
    if (id === undefined || id === null) {
      throw new RoderTransportError("requests require a non-null id");
    }
    const key = String(id);
    const promise = new Promise<JsonRpcResponse<R>>((resolve, reject) => {
      const abort = () => {
        this.pending.delete(key);
        reject(new DOMException("Request aborted", "AbortError"));
      };
      options.signal?.addEventListener("abort", abort, { once: true });
      this.pending.set(key, {
        resolve: (response) => resolve(response as JsonRpcResponse<R>),
        reject,
        cleanup: () => options.signal?.removeEventListener("abort", abort),
      });
    });
    this.socket.send(JSON.stringify(request));
    return promise;
  }

  notifications(): AsyncIterable<JsonRpcNotification> {
    return this.notificationHub.subscribe();
  }

  close(): void {
    this.socket.close();
    this.notificationHub.close();
  }

  private handleMessage(data: string): void {
    const message = JSON.parse(data) as JsonRpcResponse | JsonRpcNotification;
    if ("id" in message) {
      const key = String(message.id);
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
    for (const pending of this.pending.values()) {
      pending.cleanup();
      pending.reject(error);
    }
    this.pending.clear();
  }
}

type PendingResponse = {
  resolve: (response: JsonRpcResponse) => void;
  reject: (error: Error) => void;
  cleanup: () => void;
};

/**
 * Fans notifications out to every active subscriber. The agent's callback loop
 * and each RoderRun stream subscribe independently; a single shared queue
 * would deliver each notification to only one of them. Notifications pushed
 * while no subscriber exists are buffered and replayed to the next subscriber.
 */
class NotificationHub {
  private readonly subscribers = new Set<AsyncQueue<JsonRpcNotification>>();
  private backlog: JsonRpcNotification[] = [];
  private closed = false;

  push(notification: JsonRpcNotification): void {
    if (this.closed) {
      return;
    }
    if (this.subscribers.size === 0) {
      this.backlog.push(notification);
      return;
    }
    for (const queue of this.subscribers) {
      queue.push(notification);
    }
  }

  close(): void {
    this.closed = true;
    this.backlog = [];
    for (const queue of this.subscribers) {
      queue.close();
    }
    this.subscribers.clear();
  }

  subscribe(): AsyncIterable<JsonRpcNotification> {
    const queue = new AsyncQueue<JsonRpcNotification>();
    if (this.closed) {
      queue.close();
      return queue;
    }
    for (const notification of this.backlog.splice(0)) {
      queue.push(notification);
    }
    this.subscribers.add(queue);
    const unsubscribe = () => {
      this.subscribers.delete(queue);
      queue.close();
    };
    return {
      [Symbol.asyncIterator]: () => {
        const inner = queue[Symbol.asyncIterator]();
        return {
          next: () => inner.next(),
          return: async (): Promise<IteratorResult<JsonRpcNotification>> => {
            unsubscribe();
            return { value: undefined, done: true };
          },
        };
      },
    };
  }
}

class AsyncQueue<T> implements AsyncIterable<T> {
  private readonly values: T[] = [];
  private readonly waiters: Array<(result: IteratorResult<T>) => void> = [];
  private done = false;

  push(value: T): void {
    const waiter = this.waiters.shift();
    if (waiter) {
      waiter({ value, done: false });
    } else {
      this.values.push(value);
    }
  }

  close(): void {
    this.done = true;
    for (const waiter of this.waiters.splice(0)) {
      waiter({ value: undefined, done: true });
    }
  }

  [Symbol.asyncIterator](): AsyncIterator<T> {
    return {
      next: () => {
        const value = this.values.shift();
        if (value !== undefined) {
          return Promise.resolve({ value, done: false });
        }
        if (this.done) {
          return Promise.resolve({ value: undefined, done: true });
        }
        return new Promise((resolve) => this.waiters.push(resolve));
      },
    };
  }
}

function defaultWebSocketFactory(
  url: string,
  protocols: string[],
  options: { headers?: Record<string, string> },
): WebSocketLike {
  const WebSocketCtor = globalThis.WebSocket as unknown as {
    new (url: string, protocols: string[], options?: { headers?: Record<string, string> }): WebSocketLike;
  };
  if (!WebSocketCtor) {
    throw new RoderTransportError("global WebSocket is unavailable");
  }
  return new WebSocketCtor(url, protocols, options);
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) {
    throw new DOMException("Request aborted", "AbortError");
  }
}

function abortable<T>(promise: Promise<T>, signal: AbortSignal | undefined): Promise<T> {
  if (!signal) {
    return promise;
  }
  return Promise.race([
    promise,
    new Promise<T>((_, reject) => {
      signal.addEventListener("abort", () => reject(new DOMException("Request aborted", "AbortError")), {
        once: true,
      });
    }),
  ]);
}

function isNotification(
  message: JsonRpcResponse | JsonRpcNotification,
): message is JsonRpcNotification {
  return "method" in message;
}
