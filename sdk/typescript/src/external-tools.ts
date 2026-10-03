import type { RoderRpcClient } from "./client.js";
import type { ExternalToolCall } from "./protocol.js";
import type { JsonRpcNotification } from "./transports.js";

/** Opaque server-issued ownership of one hosted thread's external tool callbacks. */
export interface ToolExecutorLease {
  threadId: string;
  leaseId: string;
  generation: number;
  contractVersion: 1;
}

export interface ExternalToolExecutionContext {
  threadId: string;
  turnId: string;
  requestId: string;
  signal: AbortSignal;
}

export interface ExternalToolExecutionResult {
  output: string;
  isError?: boolean;
}

export interface ToolExecutionState {
  threadId: string;
  turnId: string;
  requestId: string;
  toolName: string;
  state: string;
  isError: boolean;
}

type Execute = (call: ExternalToolCall, context: ExternalToolExecutionContext) =>
  ExternalToolExecutionResult | Promise<ExternalToolExecutionResult>;

function matches(value: unknown, lease: ToolExecutorLease): boolean {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<ToolExecutorLease>;
  return candidate.threadId === lease.threadId && candidate.leaseId === lease.leaseId &&
    candidate.generation === lease.generation && candidate.contractVersion === 1;
}

/** Feed live notifications from the host's existing loop; this starts no model loop.
 * Close before reconnect/navigation. Callbacks must check signal before committing effects.
 * Rebinding never replays pending or historical requests.
 */
export class ExternalToolExecutor {
  private active = true;
  private readonly seen = new Set<string>();
  private readonly pending = new Map<string, {turnId: string; controller: AbortController}>();
  private readonly endedTurns = new Set<string>();

  private constructor(
    private readonly client: RoderRpcClient,
    readonly lease: ToolExecutorLease,
    private readonly execute: Execute,
  ) {}

  static async bind(client: RoderRpcClient, threadId: string, execute: Execute,
    options: { takeover?: boolean } = {}): Promise<ExternalToolExecutor> {
    const result = await client.call<"tools/bind_executor", unknown, {executor: ToolExecutorLease}>(
      "tools/bind_executor", {threadId, takeover: options.takeover ?? false});
    if (result.executor.threadId !== threadId || result.executor.contractVersion !== 1) {
      throw new Error("Unsupported external tool executor contract");
    }
    return new ExternalToolExecutor(client, result.executor, execute);
  }

  /** Resolves after this notification has been handled; independent calls may run in parallel. */
  async handle(notification: JsonRpcNotification): Promise<void> {
    const params = notification.params as Record<string, unknown> | undefined;
    if (!params || !this.active) return;
    if (notification.method === "tools/executorRevoked" && matches(params.executor, this.lease)) {
      this.stop();
      return;
    }
    if (params.threadId !== this.lease.threadId) return;
    if (notification.method === "thread/toolExecutionResolved" && typeof params.requestId === "string") {
      this.pending.get(params.requestId)?.controller.abort();
      return;
    }
    if (notification.method === "turn/completed") {
      const turn = params.turn as {id?: string} | undefined;
      if (turn?.id) {
        if (this.endedTurns.size >= 4096) { await this.close(); return; }
        this.endedTurns.add(turn.id);
        for (const pending of this.pending.values()) {
          if (pending.turnId === turn.id) pending.controller.abort();
        }
      }
      return;
    }
    if (notification.method !== "thread/toolExecutionRequested" || !matches(params.executor, this.lease) ||
        typeof params.requestId !== "string" || typeof params.turnId !== "string" ||
        this.seen.has(params.requestId) || this.endedTurns.has(params.turnId)) return;
    const call = params.call as ExternalToolCall | undefined;
    if (!call || typeof call.name !== "string" || typeof call.id !== "string") return;
    // Never evict deduplication ids during a binding: an old notification must not run again.
    if (this.seen.size >= 4096) { await this.close(); return; }
    this.seen.add(params.requestId);
    const controller = new AbortController();
    this.pending.set(params.requestId, {turnId: params.turnId, controller});
    try {
      let result: ExternalToolExecutionResult;
      try {
        result = await this.execute(call, {threadId: this.lease.threadId, turnId: params.turnId,
          requestId: params.requestId, signal: controller.signal});
      } catch (error) {
        result = {output: String(error), isError: true};
      }
      if (!this.active || controller.signal.aborted) return;
      const resolution = await this.client.call<"tools/resolve", unknown, {resolved: boolean}>("tools/resolve", {executor: this.lease, turnId: params.turnId,
        requestId: params.requestId, output: result.output, isError: result.isError ?? false});
      if (!resolution.resolved) throw new Error("External tool request is terminal; read current state before continuing");
    } finally {
      this.pending.delete(params.requestId);
    }
  }

  async read(requestId: string): Promise<ToolExecutionState | null> {
    const result = await this.client.call<"tools/execution_read", unknown, {execution: ToolExecutionState | null}>(
      "tools/execution_read", {executor: this.lease, requestId});
    return result.execution;
  }

  /** Call synchronously when transport drops, before any retry or new binding. */
  stop(): void {
    this.active = false;
    for (const pending of this.pending.values()) pending.controller.abort();
    this.pending.clear();
  }

  get isActive(): boolean { return this.active; }

  async close(): Promise<void> {
    this.stop();
    await this.client.call("tools/unbind_executor", {executor: this.lease}).catch(() => {});
  }
}
