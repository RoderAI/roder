import { RoderRpcError } from "./errors.js";
import type { AppServerMethod, JsonRpcId, JsonRpcRequest, JsonRpcResponse } from "./types.generated.js";
import { appServerManifest } from "./types.generated.js";
import type { RequestOptions, RoderTransport } from "./transports.js";

export type MethodHelpers = {
  [M in AppServerMethod]: <P = unknown, R = unknown>(params?: P, options?: RequestOptions) => Promise<R>;
};

export class RoderRpcClient {
  readonly methods: MethodHelpers;
  private nextId = 1;

  constructor(private readonly transport: RoderTransport) {
    this.methods = Object.fromEntries(
      appServerManifest.methods.map((spec) => [
        spec.method,
        (params?: unknown, options?: RequestOptions) => this.call(spec.method, params, options),
      ]),
    ) as MethodHelpers;
  }

  async call<M extends AppServerMethod, P = unknown, R = unknown>(
    method: M,
    params?: P,
    options: RequestOptions = {},
  ): Promise<R> {
    const id = this.allocateId();
    const request: JsonRpcRequest<M, P> = {
      jsonrpc: "2.0",
      id,
      method,
      ...(params === undefined ? {} : { params }),
    };
    const response = await this.rawRequest<M, P, R>(request, options);
    if (response.error) {
      throw new RoderRpcError(response.error, method, response.id);
    }
    return response.result as R;
  }

  rawRequest<M extends AppServerMethod, P = unknown, R = unknown>(
    request: JsonRpcRequest<M, P>,
    options?: RequestOptions,
  ): Promise<JsonRpcResponse<R>> {
    return this.transport.request<M, P, R>(request, options);
  }

  notifications() {
    return this.transport.notifications();
  }

  close(): Promise<void> | void {
    return this.transport.close();
  }

  private allocateId(): JsonRpcId {
    return this.nextId++;
  }
}
