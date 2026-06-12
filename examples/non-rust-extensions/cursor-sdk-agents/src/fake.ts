/**
 * In-process fake of the `@cursor/sdk` surface used by this extension.
 *
 * Selected with `CURSOR_SDK_FAKE=1` (offline node tests and the Rust
 * app-server e2e). Behaviour mirrors the documented SDK contract: cloud
 * agents get `bc-` ids, runs stream `status`/`assistant`/`tool_call`
 * events, results carry `requestId`/`result`/`git` metadata, and
 * `Agent.resume` reattaches by id within this process.
 *
 * Prompt sentinels for failure-path tests:
 * - `FAKE_SDK_RUN_ERROR`: the run finishes with `status: "error"`.
 * - `FAKE_SDK_THROW`: `agent.send` throws an error that embeds key-shaped
 *   secrets, to prove redaction.
 * - `FAKE_SDK_SLOW`: the stream pauses between events so cancellation has
 *   a window to land.
 */

import type {
  SdkAgent,
  SdkMessage,
  SdkModule,
  SdkRun,
  SdkRunResult,
} from "./sdk.js";

interface FakeAgentState {
  agentId: string;
  model: string | undefined;
  cloud: Record<string, unknown> | undefined;
  prompts: string[];
}

let agentCounter = 0;
const agents = new Map<string, FakeAgentState>();

function makeRun(state: FakeAgentState, prompt: string): SdkRun {
  const requestId = `request-${state.agentId}-${state.prompts.length}`;
  const runId = `run-${state.agentId}-${state.prompts.length}`;
  let cancelled = false;

  const autoCreatePr = Boolean(state.cloud?.autoCreatePR);
  const repos = (state.cloud?.repos ?? []) as Array<{ url?: string }>;
  const repoUrl = repos[0]?.url ?? "https://github.com/example-org/example-repo";

  const result: SdkRunResult = {
    id: runId,
    requestId,
    status: prompt.includes("FAKE_SDK_RUN_ERROR") ? "error" : "finished",
    result: `Fake cloud agent completed: ${prompt}`,
    model: state.model ? { id: state.model } : undefined,
    durationMs: 42,
    git: {
      branches: [
        {
          repoUrl,
          branch: `cursor/fake-${state.agentId}`,
          ...(autoCreatePr
            ? { prUrl: `https://github.com/example-org/example-repo/pull/7` }
            : {}),
        },
      ],
    },
  };

  const slow = prompt.includes("FAKE_SDK_SLOW");
  const pause = async (): Promise<void> => {
    if (slow) {
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
  };

  return {
    requestId,
    async *stream(): AsyncGenerator<SdkMessage, void, void> {
      const base = { agent_id: state.agentId, run_id: runId };
      yield { type: "status", status: "CREATING", message: "provisioning VM", ...base };
      await pause();
      yield { type: "status", status: "RUNNING", ...base };
      await pause();
      if (cancelled) {
        yield { type: "status", status: "CANCELLED", ...base };
        return;
      }
      yield {
        type: "tool_call",
        call_id: "call-1",
        name: "read_file",
        status: "completed",
        args: { secret: "crsr_fake_args_should_never_leak" },
        ...base,
      };
      yield {
        type: "assistant",
        message: { role: "assistant", content: [{ type: "text", text: "working on it" }] },
        ...base,
      };
      if (cancelled) {
        yield { type: "status", status: "CANCELLED", ...base };
        return;
      }
      yield {
        type: "status",
        status: result.status === "error" ? "ERROR" : "FINISHED",
        ...base,
      };
    },
    async wait(): Promise<SdkRunResult> {
      if (cancelled) {
        return { ...result, status: "cancelled", result: undefined };
      }
      return result;
    },
    async cancel(): Promise<void> {
      cancelled = true;
    },
  };
}

function makeAgent(state: FakeAgentState): SdkAgent {
  return {
    agentId: state.agentId,
    async send(prompt: string): Promise<SdkRun> {
      if (prompt.includes("FAKE_SDK_THROW")) {
        throw new Error(
          "fake SDK rejected the request for key crsr_supersecretvalue1234 (Bearer abc.def.ghi)",
        );
      }
      state.prompts.push(prompt);
      return makeRun(state, prompt);
    },
    close(): void {},
  };
}

export function createFakeSdkModule(): SdkModule {
  return {
    Agent: {
      async create(options: Record<string, unknown>): Promise<SdkAgent> {
        if (typeof options.apiKey !== "string" || options.apiKey.length === 0) {
          throw new Error("apiKey is required");
        }
        const cloud = options.cloud as Record<string, unknown> | undefined;
        agentCounter += 1;
        const agentId = cloud ? `bc-fake-${agentCounter}` : `agent-fake-${agentCounter}`;
        const model = (options.model as { id?: string } | undefined)?.id;
        const state: FakeAgentState = { agentId, model, cloud, prompts: [] };
        agents.set(agentId, state);
        return makeAgent(state);
      },
      async resume(agentId: string): Promise<SdkAgent> {
        const state = agents.get(agentId);
        if (!state) {
          throw new Error(`unknown agent ${agentId}`);
        }
        return makeAgent(state);
      },
    },
    Cursor: {
      models: {
        async list(): Promise<Array<{ id: string }>> {
          return [{ id: "composer-2.5" }, { id: "gpt-5.5" }];
        },
      },
    },
  };
}

/** Test helper: reset fake agent state between tests. */
export function resetFakeSdk(): void {
  agentCounter = 0;
  agents.clear();
}
