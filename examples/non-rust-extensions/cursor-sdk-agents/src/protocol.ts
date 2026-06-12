/**
 * Newline-delimited JSON-RPC 2.0 stdio plumbing for Roder process
 * extensions, plus the typed payload shapes this extension exchanges with
 * the Rust host.
 *
 * The Rust host owns request ids; this module reads host messages from
 * stdin, routes them to a handler, and writes responses/notifications to
 * stdout. Diagnostics go to stderr only — stdout is reserved for protocol
 * frames.
 *
 * Wrapper payloads (dispatch/execute params, event notifications) use
 * camelCase keys; the canonical Roder DTOs they carry (SubagentRequest,
 * SubagentResult, TaskSpec, ...) use snake_case keys, matching the Rust
 * serde shapes in `roder_api::process_extension`.
 */

import * as readline from "node:readline";

export const PROTOCOL_VERSION = "0.2.0";

export const METHOD_INITIALIZE = "extension/initialize";
export const METHOD_SUBAGENTS_DEFINITIONS = "subagents/definitions";
export const METHOD_SUBAGENTS_DISPATCH = "subagents/dispatch";
export const METHOD_SUBAGENTS_EVENT = "subagents/event";
export const METHOD_SUBAGENTS_CANCEL = "subagents/cancel";
export const METHOD_TASKS_SPEC = "tasks/spec";
export const METHOD_TASKS_EXECUTE = "tasks/execute";
export const METHOD_TASKS_EVENT = "tasks/event";
export const METHOD_TASKS_CANCEL = "tasks/cancel";
export const METHOD_EVENTS_HANDLE = "events/handle";
export const METHOD_EXTENSION_EVENT = "extension/event";
export const METHOD_SHUTDOWN = "extension/shutdown";

/** Canonical Roder subagent request (snake_case fields). */
export interface SubagentRequest {
  description: string;
  prompt: string;
  subagent_type?: string | null;
  model?: string | null;
  tools?: string[] | null;
  inputs?: Record<string, unknown> | null;
  timeout_seconds?: number | null;
  [key: string]: unknown;
}

/** Canonical Roder subagent result (snake_case fields). */
export interface SubagentResult {
  thread_id: string;
  turn_id: string;
  agent_type: string;
  model: string | null;
  final_message: string;
  usage: null;
  exit_reason: "completed" | "max_turns" | "timeout" | "cancelled" | "failed";
  metadata: Record<string, unknown>;
}

/** Canonical Roder subagent definition (snake_case fields). */
export interface SubagentDefinition {
  agent_type: string;
  description: string;
  tools: string[];
  model: string | null;
  system_prompt: string | null;
  permission_mode: "read_only" | "default" | "auto_edit";
  max_turns: number | null;
  max_result_chars: number | null;
}

/** Canonical Roder task spec (snake_case fields). */
export interface TaskSpec {
  kind: string;
  description: string;
  input_schema: Record<string, unknown>;
  default_timeout_seconds?: number | null;
  metadata?: Record<string, unknown>;
}

export interface TaskExecutionResult {
  exit_code?: number | null;
  payload: Record<string, unknown>;
}

export type SubagentEvent =
  | { type: "status"; status: string; detail?: string }
  | { type: "completed"; result: SubagentResult }
  | { type: "failed"; error: string };

export type TaskEvent =
  | { type: "output"; stream: "stdout" | "stderr" | "log"; chunk: string }
  | { type: "completed"; result: TaskExecutionResult }
  | { type: "failed"; error: string };

export interface SubagentDispatchParams {
  dispatcherId: string;
  dispatchId: string;
  parentThreadId: string;
  parentTurnId: string;
  request: SubagentRequest;
}

export interface TaskExecuteParams {
  executorId: string;
  executionId: string;
  taskId: string;
  threadId?: string;
  turnId?: string;
  workspaceRoot?: string;
  input: Record<string, unknown>;
}

/** FNV-1a 64-bit hex checksum matching the Rust host implementation. */
export function fnv1aChecksum(data: Buffer): string {
  const prime = 0x0000_0100_0000_01b3n;
  const mask = 0xffff_ffff_ffff_ffffn;
  let hash = 0xcbf2_9ce4_8422_2325n;
  for (const byte of data) {
    hash ^= BigInt(byte);
    hash = (hash * prime) & mask;
  }
  return hash.toString(16).padStart(16, "0");
}

/**
 * Strips Cursor key material and bearer headers from a message before it
 * leaves the child. Applied to every error and event detail.
 */
export function redactSecrets(message: string): string {
  let redacted = message
    .replace(/crsr_[A-Za-z0-9]+/g, "crsr_[REDACTED]")
    .replace(/Bearer\s+\S+/g, "Bearer [REDACTED]");
  const apiKey = process.env.CURSOR_API_KEY;
  if (apiKey && apiKey.length > 0) {
    redacted = redacted.split(apiKey).join("[REDACTED]");
  }
  return redacted;
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    return redactSecrets(error.message);
  }
  return redactSecrets(String(error));
}

export interface RpcWriter {
  reply(id: unknown, result: unknown): void;
  replyError(id: unknown, message: string): void;
  notify(method: string, params: unknown): void;
}

/** Sentinel a handler returns after replying to the request itself. */
export const ALREADY_REPLIED: unique symbol = Symbol("ALREADY_REPLIED");

export class ShutdownRequested extends Error {
  constructor() {
    super("shutdown requested");
  }
}

export type RpcHandler = (
  method: string,
  params: Record<string, unknown>,
  rpc: RpcWriter,
  msgId: unknown,
) => Promise<unknown | typeof ALREADY_REPLIED> | unknown | typeof ALREADY_REPLIED;

/**
 * Blocking stdio JSON-RPC loop. `handler(method, params, rpc, id)` returns
 * a result value for requests (or ALREADY_REPLIED after acking itself);
 * notifications return undefined implicitly.
 */
export class StdioRpc implements RpcWriter {
  private readonly out: NodeJS.WritableStream;

  constructor(out: NodeJS.WritableStream = process.stdout) {
    this.out = out;
  }

  private write(message: Record<string, unknown>): void {
    this.out.write(`${JSON.stringify(message)}\n`);
  }

  reply(id: unknown, result: unknown): void {
    this.write({ jsonrpc: "2.0", id, result });
  }

  replyError(id: unknown, message: string): void {
    this.write({
      jsonrpc: "2.0",
      id,
      error: { code: -32000, message: redactSecrets(message) },
    });
  }

  notify(method: string, params: unknown): void {
    this.write({ jsonrpc: "2.0", method, params });
  }

  async run(handler: RpcHandler, input: NodeJS.ReadableStream = process.stdin): Promise<void> {
    const lines = readline.createInterface({ input, crlfDelay: Infinity });
    for await (const line of lines) {
      const trimmed = line.trim();
      if (trimmed.length === 0) {
        continue;
      }
      let message: Record<string, unknown>;
      try {
        message = JSON.parse(trimmed) as Record<string, unknown>;
      } catch {
        process.stderr.write("dropped non-JSON stdin line\n");
        continue;
      }
      const method = typeof message.method === "string" ? message.method : "";
      const msgId = message.id;
      const params = (message.params ?? {}) as Record<string, unknown>;
      try {
        const result = await handler(method, params, this, msgId);
        if (msgId !== undefined && result !== ALREADY_REPLIED) {
          this.reply(msgId, result ?? {});
        }
      } catch (error) {
        if (error instanceof ShutdownRequested) {
          if (msgId !== undefined) {
            this.reply(msgId, {});
          }
          return;
        }
        if (msgId !== undefined) {
          this.replyError(msgId, errorMessage(error));
        } else {
          process.stderr.write(`notification ${method} failed: ${errorMessage(error)}\n`);
        }
      }
    }
  }
}
