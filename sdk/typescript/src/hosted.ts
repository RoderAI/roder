// Hosted multi-tenant connection helpers (roadmap phase 72, Task 6).
//
// Hosted Roder authenticates at the WebSocket handshake with a bearer
// credential (static key or `rk_sa_*` service-account key) in the
// `Authorization` header — query-string credentials are always rejected by
// the gateway. `HostedClient` wraps the standard RPC client with typed
// helpers for `hosted/*` methods; raw JSON-RPC access stays available for
// forward-compatible hosted methods via `client.rawRequest`.
//
// Token refresh/reconnect: connections are authenticated once at handshake
// time, so refreshing a token means reconnecting. `reconnect()` builds a
// fresh transport using the token provider. Requests that were in flight
// when a connection dropped fail with a transport error and are NEVER
// replayed automatically — mutating requests must be retried by the
// caller, who knows whether the operation is idempotent.

import { RoderRpcClient } from "./client.js";
import { WebSocketTransport, type WebSocketFactory } from "./transports.js";

export interface HostedConnectOptions {
  /** Gateway URL, e.g. `wss://roder.example.com`. */
  url: string;
  /** Static bearer token or service-account key. */
  token?: string;
  /**
   * Called whenever a (re)connection needs a credential; takes precedence
   * over `token`. Use this for short-lived externally-issued tokens.
   */
  tokenProvider?: () => string | Promise<string>;
  /** Extra auth headers supplied by an external auth layer. */
  headers?: Record<string, string>;
  protocols?: string[];
  webSocketFactory?: WebSocketFactory;
}

export interface HostedWhoami {
  tenant: { tenantId: string; displayName?: string };
  principal: Record<string, unknown>;
  role: string;
  scopes: string[];
}

export interface HostedServiceAccountKey {
  keyId: string;
  /** Returned exactly once; the gateway stores only a hash. */
  token: string;
}

export interface HostedHookDefinition {
  id: string;
  scope: "tenant" | "system";
  eventKinds: string[];
  url: string;
  /** `env:NAME` reference; raw secrets are rejected by the gateway. */
  signingSecretRef?: string;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
}

export class HostedClient {
  private constructor(
    private readonly options: HostedConnectOptions,
    public client: RoderRpcClient,
  ) {}

  /** Connects and authenticates against a hosted Roder gateway. */
  static async connect(options: HostedConnectOptions): Promise<HostedClient> {
    const client = await hostedRpcClient(options);
    return new HostedClient(options, client);
  }

  /**
   * Re-authenticates with a fresh credential from the token provider and
   * replaces the underlying connection. In-flight requests on the old
   * connection fail; nothing is replayed.
   */
  async reconnect(): Promise<void> {
    const next = await hostedRpcClient(this.options);
    const previous = this.client;
    this.client = next;
    await previous.close();
  }

  whoami(): Promise<HostedWhoami> {
    return this.client.call("hosted/whoami", {});
  }

  createServiceAccount(displayName: string): Promise<HostedServiceAccountKey> {
    return this.client.call("hosted/service_accounts/create", { displayName });
  }

  revokeServiceAccount(keyId: string): Promise<{ revoked: boolean }> {
    return this.client.call("hosted/service_accounts/revoke", { keyId });
  }

  listHooks(): Promise<{ hooks: HostedHookDefinition[] }> {
    return this.client.call("hosted/hooks/list", {});
  }

  createHook(hook: HostedHookDefinition): Promise<{ hook: HostedHookDefinition }> {
    return this.client.call("hosted/hooks/create", { hook });
  }

  deleteHook(hookId: string): Promise<{ deleted: boolean }> {
    return this.client.call("hosted/hooks/delete", { hookId });
  }

  auditList(): Promise<{ records: Record<string, unknown>[] }> {
    return this.client.call("hosted/audit/list", {});
  }

  notifications() {
    return this.client.notifications();
  }

  close(): Promise<void> | void {
    return this.client.close();
  }
}

async function hostedRpcClient(options: HostedConnectOptions): Promise<RoderRpcClient> {
  const token = options.tokenProvider ? await options.tokenProvider() : options.token;
  if (!token && !options.headers?.Authorization && !options.headers?.authorization) {
    throw new Error(
      "hosted connections require a token, tokenProvider, or an Authorization header",
    );
  }
  const transport = new WebSocketTransport({
    url: options.url,
    token,
    protocols: options.protocols,
    webSocketFactory: options.webSocketFactory,
    headers: options.headers,
  });
  return new RoderRpcClient(transport);
}
