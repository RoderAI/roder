import assert from "node:assert/strict";
import test from "node:test";
import {
  categorizeRoderError,
  RoderRpcError,
  RoderTransportError,
  type RoderErrorCategory,
} from "../src/index.js";

/**
 * Messages mirror real server output: provider passthroughs use the shared
 * `<Provider> error <status>: <body>` format, runtime context-window and tool
 * validation strings come from roder-core, and -32602/-32601/-32700 are the
 * app-server's JSON-RPC protocol errors.
 */
const classificationTable: Array<{ code: number; message: string; category: RoderErrorCategory }> = [
  { code: -32000, message: "provider authentication is not configured", category: "auth" },
  {
    code: -32000,
    message:
      'Anthropic error 401: {"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}',
    category: "auth",
  },
  { code: -32000, message: "Gemini error 403: permission denied", category: "auth" },
  {
    code: -32000,
    message:
      "Your input exceeds the context window of this model. Please adjust your input and try again.",
    category: "context_length",
  },
  {
    code: -32000,
    message: "OpenRouter context length exceeded: maximum context length is 128000 tokens.",
    category: "context_length",
  },
  {
    code: -32000,
    message:
      'Anthropic error 400: {"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 250000 tokens > 200000 maximum"}}',
    category: "context_length",
  },
  {
    code: -32000,
    message: "invalid tool call arguments: missing field `path`",
    category: "invalid_tool_input",
  },
  { code: -32000, message: "Anthropic error 429: rate limited", category: "provider" },
  { code: -32000, message: "OpenAI Responses error 500: internal error", category: "provider" },
  { code: -32000, message: "OpenAI Chat Completions error 529: overloaded", category: "provider" },
  { code: -32602, message: "Invalid params: missing field `threadId`", category: "invalid_request" },
  { code: -32602, message: "Missing params", category: "invalid_request" },
  { code: -32601, message: "Method not found", category: "invalid_request" },
  { code: -32700, message: "Parse error", category: "invalid_request" },
  { code: -32000, message: "No memory store is registered", category: "unknown" },
  { code: -32004, message: "thread/start denied by policy: blocked", category: "unknown" },
];

test("rpc error classification table", () => {
  for (const { code, message, category } of classificationTable) {
    const error = new RoderRpcError({ code, message }, "thread/start", "req-1");
    assert.equal(error.category, category, `code=${code} message=${message}`);
  }
});

test("categorizeRoderError maps thrown values to categories", () => {
  assert.equal(
    categorizeRoderError(new RoderTransportError("app-server exited code=1 signal=null")),
    "transport",
  );
  assert.equal(
    categorizeRoderError(new RoderRpcError({ code: -32602, message: "Invalid params: bad" }, "m", 1)),
    "invalid_request",
  );
  assert.equal(categorizeRoderError(new Error("plain")), "unknown");
  assert.equal(categorizeRoderError("not an error"), "unknown");
});
