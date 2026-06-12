/**
 * The `cursor-cloud` subagent dispatcher: maps a canonical Roder
 * `SubagentRequest` onto one remote Cursor cloud agent run and streams
 * `subagents/event` notifications until the terminal result.
 */

import {
  ActiveRunTable,
  awaitCloudAgent,
  parseCloudAgentInput,
  startCloudAgent,
} from "./agents.js";
import type {
  RpcWriter,
  SubagentDefinition,
  SubagentDispatchParams,
  SubagentEvent,
  SubagentResult,
} from "./protocol.js";
import { METHOD_SUBAGENTS_EVENT, errorMessage } from "./protocol.js";
import { loadSdk } from "./sdk.js";

export const DISPATCHER_ID = "cursor-cloud";

export function dispatcherDefinitions(): SubagentDefinition[] {
  return [
    {
      agent_type: "cursor-cloud",
      description:
        "Dispatch a remote Cursor cloud agent against a GitHub repository and return its final summary. Inputs: repoUrl (required to create), startingRef, autoCreatePr, agentId (resume).",
      tools: [],
      model: null,
      system_prompt: null,
      permission_mode: "default",
      max_turns: null,
      max_result_chars: null,
    },
  ];
}

export class CursorCloudDispatcher {
  private readonly active = new ActiveRunTable();

  /**
   * Runs the dispatch after the ack has been written; emits status events
   * and exactly one terminal `completed`/`failed` event.
   */
  async dispatch(params: SubagentDispatchParams, rpc: RpcWriter): Promise<void> {
    const emit = (event: SubagentEvent): void => {
      rpc.notify(METHOD_SUBAGENTS_EVENT, { dispatchId: params.dispatchId, event });
    };

    try {
      const input = parseCloudAgentInput(params.request.inputs ?? {}, {
        prompt: params.request.prompt,
        model: params.request.model ?? undefined,
      });
      const sdk = await loadSdk();
      const started = await startCloudAgent(sdk, input);
      this.active.insert(params.dispatchId, { run: started.run, agentId: started.agentId });
      emit({
        type: "status",
        status: "SUBMITTED",
        detail: `cloud agent ${started.agentId}`,
      });

      const outcome = await awaitCloudAgent(started, (status, detail) => {
        emit({ type: "status", status, ...(detail ? { detail } : {}) });
      });
      this.active.remove(params.dispatchId);

      if (outcome.status === "error") {
        emit({
          type: "failed",
          error: `cloud agent ${outcome.agentId} run ${outcome.runId} finished with status error`,
        });
        return;
      }

      const result: SubagentResult = {
        thread_id: outcome.agentId,
        turn_id: outcome.requestId || outcome.runId,
        agent_type: "cursor-cloud",
        model: outcome.model,
        final_message: outcome.resultText,
        usage: null,
        exit_reason: outcome.status === "cancelled" ? "cancelled" : "completed",
        metadata: {
          agentId: outcome.agentId,
          requestId: outcome.requestId,
          runId: outcome.runId,
          status: outcome.status,
          prUrls: outcome.prUrls,
          branches: outcome.branches,
          durationMs: outcome.durationMs,
          resumed: outcome.resumed,
        },
      };
      emit({ type: "completed", result });
    } catch (error) {
      this.active.remove(params.dispatchId);
      emit({ type: "failed", error: errorMessage(error) });
    }
  }

  async cancel(dispatchId: string): Promise<boolean> {
    return this.active.cancel(dispatchId);
  }
}
