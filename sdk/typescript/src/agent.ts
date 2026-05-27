import { RoderRpcClient } from "./client.js";
import { RoderRun } from "./run.js";
import {
  InMemoryTransport,
  LocalProcessTransport,
  WebSocketTransport,
  type RoderTransport,
} from "./transports.js";
import type { EventMode } from "./events.js";

export interface RoderAgentOptions {
  local?: {
    cwd?: string;
    command?: string;
    args?: string[];
    env?: NodeJS.ProcessEnv;
  };
  remote?: {
    url: string;
    token?: string;
    protocols?: string[];
  };
  transport?: RoderTransport;
  cwd?: string;
  model?: {
    provider?: string;
    id?: string;
  };
  threadId?: string;
  approvals?: RoderApprovals;
  eventMode?: EventMode;
}

export interface RoderApprovals {
  onToolApproval?(request: unknown): Promise<ApprovalDecision> | ApprovalDecision;
  onUserInput?(request: unknown): Promise<UserInputDecision> | UserInputDecision;
  onPlanExit?(request: unknown): Promise<PlanExitDecision> | PlanExitDecision;
}

export interface ApprovalDecision {
  approved: boolean;
}

export interface UserInputDecision {
  answers: unknown;
}

export interface PlanExitDecision {
  approved: boolean;
}

export class RoderAgent {
  readonly client: RoderRpcClient;
  private threadId: string | undefined;
  private callbackLoopStarted = false;

  private constructor(
    private readonly transport: RoderTransport,
    private readonly options: RoderAgentOptions,
  ) {
    this.client = new RoderRpcClient(transport);
    this.threadId = options.threadId;
  }

  static async create(options: RoderAgentOptions): Promise<RoderAgent> {
    const transport = resolveTransport(options);
    const agent = new RoderAgent(transport, options);
    agent.startCallbackLoop();
    return agent;
  }

  async send(input: string | Array<Record<string, unknown>>, options: { eventMode?: EventMode } = {}): Promise<RoderRun> {
    const threadId = this.threadId ?? (await this.startThread());
    this.threadId = threadId;
    const result = (await this.client.call("turn/start", {
      threadId,
      input: normalizeInput(input),
    })) as Record<string, unknown>;
    const turnId = extractId(result, "turn") ?? extractString(result, "turnId") ?? extractString(result, "id");
    if (!turnId) {
      throw new Error("turn/start response did not include a turn id");
    }
    return new RoderRun(this.client, threadId, turnId, {
      eventMode: options.eventMode ?? this.options.eventMode,
    });
  }

  async listModels(): Promise<unknown> {
    return this.client.call("model/list");
  }

  async listProviders(): Promise<unknown> {
    return this.client.call("providers/list");
  }

  async readThread(threadId = this.threadId): Promise<unknown> {
    if (!threadId) {
      throw new Error("readThread requires a thread id");
    }
    return this.client.call("thread/read", { threadId });
  }

  async listThreads(): Promise<unknown> {
    return this.client.call("thread/list");
  }

  async listTools(): Promise<unknown> {
    return this.client.call("tools/list");
  }

  async listCommands(): Promise<unknown> {
    return this.client.call("commands/list");
  }

  async close(): Promise<void> {
    await this.client.close();
  }

  private async startThread(): Promise<string> {
    const result = (await this.client.call("thread/start", {
      cwd: this.options.cwd ?? this.options.local?.cwd,
      model: this.options.model?.id,
      modelProvider: this.options.model?.provider,
    })) as Record<string, unknown>;
    const threadId = extractId(result, "thread") ?? extractString(result, "threadId") ?? extractString(result, "id");
    if (!threadId) {
      throw new Error("thread/start response did not include a thread id");
    }
    return threadId;
  }

  private startCallbackLoop(): void {
    if (this.callbackLoopStarted || !this.options.approvals) {
      return;
    }
    this.callbackLoopStarted = true;
    void (async () => {
      for await (const notification of this.transport.notifications()) {
        await this.handleCallbackNotification(notification.method, notification.params);
      }
    })();
  }

  private async handleCallbackNotification(method: string, params: unknown): Promise<void> {
    const approvals = this.options.approvals;
    if (!approvals) {
      return;
    }
    if (method === "thread/approvalRequested" && approvals.onToolApproval) {
      const decision = await approvals.onToolApproval(params);
      await this.client.call("thread/resolve_approval", {
        approvalId: extractString(params, "approvalId"),
        approved: decision.approved,
      });
    } else if (method === "thread/userInputRequested" && approvals.onUserInput) {
      const decision = await approvals.onUserInput(params);
      await this.client.call("thread/resolve_user_input", {
        requestId: extractString(params, "requestId"),
        answers: decision.answers,
      });
    } else if (method === "thread/planExitRequested" && approvals.onPlanExit) {
      const decision = await approvals.onPlanExit(params);
      await this.client.call("thread/exit_plan", {
        requestId: extractString(params, "requestId"),
        approved: decision.approved,
      });
    }
  }
}

function resolveTransport(options: RoderAgentOptions): RoderTransport {
  if (options.transport) {
    return options.transport;
  }
  if (options.remote) {
    return new WebSocketTransport(options.remote);
  }
  if (options.local) {
    return new LocalProcessTransport({
      command: options.local.command,
      args: options.local.args,
      cwd: options.local.cwd ?? options.cwd,
      env: options.local.env,
    });
  }
  return new InMemoryTransport((request) => ({
    jsonrpc: "2.0",
    id: request.id,
    error: { code: -32000, message: "no transport configured" },
  }));
}

function normalizeInput(input: string | Array<Record<string, unknown>>): Array<Record<string, unknown>> {
  return typeof input === "string" ? [{ type: "text", text: input }] : input;
}

function extractId(value: Record<string, unknown>, key: string): string | undefined {
  const nested = value[key];
  return nested && typeof nested === "object" ? extractString(nested, "id") : undefined;
}

function extractString(value: unknown, key: string): string | undefined {
  if (!value || typeof value !== "object") {
    return undefined;
  }
  const candidate = (value as Record<string, unknown>)[key];
  return typeof candidate === "string" ? candidate : undefined;
}
