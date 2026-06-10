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
    /** When false, the app-server receives exactly `env` instead of inheriting process.env. */
    inheritEnv?: boolean;
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
    reasoning?: string;
  };
  /** Per-thread tool filter applied on top of the server's runtime allowlist. */
  toolAllowlist?: string[];
  /** Host instructions layered under the harness system prompt on every turn. */
  instructions?: string;
  /** Host-executed tools advertised to the model; calls arrive via onToolExecute. */
  externalTools?: RoderExternalTool[];
  /**
   * Binds the thread's native coding tools to a remote-runner workspace on the server. The config
   * is persisted with the thread, so secrets must reach the provider through its environment
   * (e.g. SAUNA_RUNNER_TOKEN), not this object.
   */
  runner?: RoderThreadRunner;
  /**
   * Executes a host tool call published by thread/toolExecutionRequested and replies via
   * tools/resolve. A thrown error resolves the call as an error result.
   */
  onToolExecute?(call: RoderExternalToolCall): Promise<ExternalToolResult> | ExternalToolResult;
  threadId?: string;
  workspaceId?: string;
  approvals?: RoderApprovals;
  eventMode?: EventMode;
}

export interface RoderExternalTool {
  name: string;
  description: string;
  /** JSON-schema for the tool arguments. */
  parameters: Record<string, unknown>;
}

export interface RoderThreadRunner {
  /** Installed remote-runner provider id (e.g. "sauna"). */
  providerId: string;
  /** Provider-specific destination config; persisted with the thread, so no secrets. */
  config?: Record<string, unknown>;
  /** Absolute path on the runner used as the thread's coding-tool workspace root. */
  workspace: string;
}

export interface RoderExternalToolCall {
  id: string;
  name: string;
  arguments: unknown;
}

export interface ExternalToolResult {
  output: string;
  isError?: boolean;
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
    /**
     * Subscribe before turn/start: while the callback loop is active the hub
     * buffers nothing, so a turn/completed delivered in the same I/O chunk as
     * the turn/start response would be lost to a lazily subscribing run and
     * wait()/stream() would never terminate.
     */
    const notifications = this.client.notifications();
    try {
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
        notifications,
      });
    } catch (error) {
      void notifications[Symbol.asyncIterator]().return?.();
      throw error;
    }
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
    const cwd = this.options.cwd ?? this.options.local?.cwd;
    const workspaceId = this.options.workspaceId ?? (await this.resolveWorkspaceId(cwd));
    const reasoning = this.options.model?.reasoning;
    const toolAllowlist = this.options.toolAllowlist;
    const instructions = this.options.instructions;
    const externalTools = this.options.externalTools;
    const runner = this.options.runner;
    const result = (await this.client.call("thread/start", {
      cwd,
      model: this.options.model?.id,
      modelProvider: this.options.model?.provider,
      ...(reasoning === undefined ? {} : { reasoning }),
      ...(toolAllowlist === undefined ? {} : { toolAllowlist }),
      ...(instructions === undefined ? {} : { developerInstructions: instructions }),
      ...(externalTools === undefined ? {} : { externalTools }),
      ...(runner === undefined ? {} : { runner }),
      workspaceId,
    })) as Record<string, unknown>;
    const threadId = extractId(result, "thread") ?? extractString(result, "threadId") ?? extractString(result, "id");
    if (!threadId) {
      throw new Error("thread/start response did not include a thread id");
    }
    return threadId;
  }

  private async resolveWorkspaceId(cwd: string | undefined): Promise<string> {
    if (!cwd) {
      throw new Error("starting a thread requires a workspaceId or a cwd to resolve one from");
    }
    const listed = (await this.client.call("workspace/list", {})) as Record<string, unknown>;
    const workspaces = Array.isArray(listed.workspaces) ? listed.workspaces : [];
    for (const workspace of workspaces) {
      if (!workspace || typeof workspace !== "object") {
        continue;
      }
      const roots = (workspace as Record<string, unknown>).roots;
      const id = extractString(workspace, "id");
      if (id && Array.isArray(roots) && roots.some((root) => extractString(root, "path") === cwd)) {
        return id;
      }
    }
    const created = (await this.client.call("workspace/create", {
      roots: [{ path: cwd }],
    })) as Record<string, unknown>;
    const workspaceId = extractId(created, "workspace");
    if (!workspaceId) {
      throw new Error("workspace/create response did not include a workspace id");
    }
    return workspaceId;
  }

  private startCallbackLoop(): void {
    if (this.callbackLoopStarted || (!this.options.approvals && !this.options.onToolExecute)) {
      return;
    }
    this.callbackLoopStarted = true;
    void (async () => {
      for await (const notification of this.transport.notifications()) {
        try {
          await this.handleCallbackNotification(notification.method, notification.params);
        } catch {
          /**
           * Resolution calls reject when the transport drops mid-callback or
           * the server refuses a stale request id; the server already times
           * the pending call out. Swallow so the loop keeps serving later
           * notifications instead of dying as an unhandled rejection that
           * crashes the host process.
           */
        }
      }
    })();
  }

  private async handleCallbackNotification(method: string, params: unknown): Promise<void> {
    /**
     * The server broadcasts every thread's notifications to every client on
     * the connection; only this agent's thread is ours to answer. Racing
     * another host's tools/resolve would silently feed it the wrong result
     * (first writer wins, the loser just sees resolved:false).
     */
    if (extractString(params, "threadId") !== this.threadId) {
      return;
    }
    if (method === "thread/toolExecutionRequested" && this.options.onToolExecute) {
      /**
       * Dispatched without awaiting: the server publishes a parallel tool
       * batch as back-to-back notifications and starts each call's
       * tools/resolve timeout at publication, so executing serially here
       * would burn call N+1's budget while call N runs. Each call resolves
       * independently; resolveExternalToolCall never rejects.
       */
      void this.resolveExternalToolCall(this.options.onToolExecute, params);
      return;
    }
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

  private async resolveExternalToolCall(
    onToolExecute: NonNullable<RoderAgentOptions["onToolExecute"]>,
    params: unknown,
  ): Promise<void> {
    const call = extractExternalToolCall(params);
    let result: ExternalToolResult;
    try {
      result = await onToolExecute(call);
    } catch (error) {
      result = { output: String(error), isError: true };
    }
    try {
      await this.client.call("tools/resolve", {
        requestId: extractString(params, "requestId"),
        output: result.output,
        isError: result.isError ?? false,
      });
    } catch {
      /**
       * The resolve rejects when the transport drops mid-call or the server
       * refuses a stale request id; the server already times the pending
       * call out. Swallow so the detached dispatch never surfaces as an
       * unhandled rejection that crashes the host process.
       */
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
      inheritEnv: options.local.inheritEnv,
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

function extractExternalToolCall(params: unknown): RoderExternalToolCall {
  const call =
    params && typeof params === "object" ? (params as Record<string, unknown>).call : undefined;
  return {
    id: extractString(call, "id") ?? "",
    name: extractString(call, "name") ?? "",
    arguments: call && typeof call === "object" ? (call as Record<string, unknown>).arguments : undefined,
  };
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
