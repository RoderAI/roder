import type { JsonRpcError, JsonRpcId } from "./types.generated.js";

export class RoderRpcError extends Error {
  readonly code: number;
  readonly data: unknown;
  readonly method: string;
  readonly requestId: JsonRpcId | undefined;

  constructor(error: JsonRpcError, method: string, requestId: JsonRpcId | undefined) {
    super(error.message);
    this.name = "RoderRpcError";
    this.code = error.code;
    this.data = error.data;
    this.method = method;
    this.requestId = requestId;
  }
}

export class RoderTransportError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "RoderTransportError";
  }
}
