import { NotificationHub, abortable, throwIfAborted, isNotification, type PendingResponse } from "./transport-support.js";
import type { AppServerMethod, JsonRpcId, JsonRpcRequest, JsonRpcResponse } from "./types.generated.js";
import { RoderTransportError } from "./errors.js";

export interface JsonRpcNotification<P = unknown> {
  jsonrpc: "2.0";
  method: string;
  params?: P;
}

export interface RoderTransport {
  /** Aborted synchronously on closure, before buffered notifications drain. */
  readonly closedSignal: AbortSignal;
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
  private readonly lifetime = new AbortController();
  readonly closedSignal = this.lifetime.signal;
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
    this.lifetime.abort();
    this.notificationHub.close();
  }
}

export interface WebSocketTransportOptions {
  url: string;
  token?: string;
  /**
   * How `token` is sent. Header auth is the backwards-compatible default;
   * browser callers should use `subprotocol` because the standard WebSocket
   * constructor cannot set handshake headers.
   */
  bearerAuth?: WebSocketBearerAuth;
  /** Extra handshake headers forwarded to a header-capable custom factory. */
  headers?: Record<string, string>;
  protocols?: string[];
  webSocketFactory?: WebSocketFactory;
}

export type WebSocketBearerAuth = "header" | "subprotocol";

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
  private readonly lifetime = new AbortController();
  readonly closedSignal = this.lifetime.signal;

  constructor(options: WebSocketTransportOptions) {
    const bearerAuth = options.bearerAuth ?? "header";
    const protocols =
      options.token && bearerAuth === "subprotocol"
        ? bearerSubprotocols(options.token, options.protocols ?? [])
        : (options.protocols ?? []);
    const headers: Record<string, string> | undefined =
      (options.token && bearerAuth === "header") || options.headers
        ? {
            ...(options.token && bearerAuth === "header"
              ? { Authorization: `Bearer ${options.token}` }
              : {}),
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
      this.lifetime.abort();
      this.rejectAll(new RoderTransportError("websocket closed"));
      this.notificationHub.close();
    });
  }

  async request<M extends AppServerMethod, P = unknown, R = unknown>(
    request: JsonRpcRequest<M, P>,
    options: RequestOptions = {},
  ): Promise<JsonRpcResponse<R>> {
    throwIfAborted(options.signal);
    if (this.closedSignal.aborted) throw new RoderTransportError("transport is closed");
    await abortable(this.opened, AbortSignal.any([this.closedSignal, ...(options.signal ? [options.signal] : [])]));
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
    this.lifetime.abort();
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
  // Standards-compliant browser WebSockets accept only URL and protocols.
  // Preserve the legacy third-argument path only when a native implementation
  // is explicitly being asked to carry headers.
  return options.headers
    ? new WebSocketCtor(url, protocols, options)
    : new WebSocketCtor(url, protocols);
}

function bearerSubprotocols(token: string, requested: string[]): string[] {
  return [
    "roder.remote.v1",
    `bearer.${token}`,
    ...requested.filter(
      (protocol) => protocol !== "roder.remote.v1" && !protocol.startsWith("bearer."),
    ),
  ];
}
