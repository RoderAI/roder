import type { JsonRpcError, JsonRpcId } from "./types.generated.js";

/**
 * Stable classification of SDK errors. Categories map to error shapes the
 * app-server actually produces today:
 *
 * - "auth": `provider authentication is not configured`
 *   (roder-core inference_routing.rs), or a provider HTTP passthrough with a
 *   401/403 status (`Anthropic error 401: …`).
 * - "context_length": context-window failures surfaced by the runtime
 *   (`Your input exceeds the context window of this model` /
 *   `context window`, roder-core runtime.rs + transcript.rs), provider
 *   passthroughs (`OpenRouter context length exceeded`,
 *   roder-ext-openai-responses), or Anthropic's 400 body
 *   (`prompt is too long`) forwarded verbatim.
 * - "invalid_tool_input": `invalid tool call arguments: …`
 *   (roder-core tool_validation.rs).
 * - "provider": any other provider HTTP passthrough in the shared
 *   `<Provider> error <status>: <body>` format (roder-ext-anthropic,
 *   roder-ext-gemini, roder-ext-openai-* providers).
 * - "invalid_request": JSON-RPC protocol codes — -32700 parse error,
 *   -32600 invalid request, -32601 method not found, -32602 invalid params
 *   (app-server `invalid_params` / `not_found` helpers).
 * - "transport": RoderTransportError (process spawn/exit, websocket close).
 * - "unknown": anything else, including -32000 internal errors with
 *   unrecognized messages.
 */
export type RoderErrorCategory =
  | "auth"
  | "context_length"
  | "invalid_tool_input"
  | "provider"
  | "invalid_request"
  | "transport"
  | "unknown";

/**
 * Shared provider HTTP passthrough format: every provider extension reports
 * upstream failures as `<Provider> error <status>: <body>`.
 */
const PROVIDER_HTTP_ERROR = /\berror (\d{3}): /;

const CONTEXT_LENGTH_PATTERNS = [
  "context window",
  "context length",
  "input exceeds",
  "prompt is too long",
];

const JSON_RPC_PROTOCOL_CODES = new Set([-32700, -32600, -32601, -32602]);

export function categorizeRpcError(code: number, message: string): RoderErrorCategory {
  const lower = message.toLowerCase();
  if (CONTEXT_LENGTH_PATTERNS.some((pattern) => lower.includes(pattern))) {
    return "context_length";
  }
  if (lower.includes("provider authentication is not configured")) {
    return "auth";
  }
  const providerStatus = PROVIDER_HTTP_ERROR.exec(lower);
  if (providerStatus) {
    const status = providerStatus[1];
    return status === "401" || status === "403" ? "auth" : "provider";
  }
  if (lower.includes("invalid tool call arguments")) {
    return "invalid_tool_input";
  }
  if (JSON_RPC_PROTOCOL_CODES.has(code)) {
    return "invalid_request";
  }
  return "unknown";
}

/** Classify any thrown value from the SDK into a RoderErrorCategory. */
export function categorizeRoderError(error: unknown): RoderErrorCategory {
  if (error instanceof RoderRpcError) {
    return error.category;
  }
  if (error instanceof RoderTransportError) {
    return "transport";
  }
  return "unknown";
}

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

  get category(): RoderErrorCategory {
    return categorizeRpcError(this.code, this.message);
  }
}

export class RoderTransportError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "RoderTransportError";
  }
}
