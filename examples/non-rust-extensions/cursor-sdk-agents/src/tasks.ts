/**
 * The `cursor-cloud-agent` task executor: create-or-resume a remote Cursor
 * cloud agent as a Roder background task. `wait: true` (default) streams
 * progress into the task log and resolves with the full outcome; `wait:
 * false` submits the run and returns ids immediately (cloud runs survive
 * the caller disconnecting).
 */

import {
  ActiveRunTable,
  awaitCloudAgent,
  parseCloudAgentInput,
  startCloudAgent,
} from "./agents.js";
import type { RpcWriter, TaskEvent, TaskExecuteParams, TaskSpec } from "./protocol.js";
import { METHOD_TASKS_EVENT, errorMessage } from "./protocol.js";
import { loadSdk } from "./sdk.js";

export const TASK_EXECUTOR_ID = "cursor-cloud-agent";

export function taskSpec(): TaskSpec {
  return {
    kind: "cursor-cloud-agent",
    description:
      "Create, resume, and await remote Cursor cloud agents through @cursor/sdk. Persist the returned agentId (bc- prefix) to resume the agent later.",
    input_schema: {
      type: "object",
      properties: {
        prompt: {
          type: "string",
          description: "Prompt submitted to the cloud agent.",
        },
        repoUrl: {
          type: "string",
          description: "https GitHub repository URL to clone (required when creating).",
        },
        startingRef: {
          type: "string",
          description: "Git ref the cloud agent starts from (default: repository default).",
        },
        autoCreatePr: {
          type: "boolean",
          description: "Ask Cursor to open a PR with the agent's changes.",
        },
        model: {
          type: "string",
          description: "Cursor model id (e.g. composer-2.5).",
        },
        agentId: {
          type: "string",
          description: "Existing cloud agent id (bc- prefix) to resume instead of creating.",
        },
        wait: {
          type: "boolean",
          description:
            "true (default): stream progress and wait for the result. false: submit and return ids immediately.",
        },
      },
      required: ["prompt"],
      additionalProperties: false,
    },
    default_timeout_seconds: 1800,
    metadata: {
      category: "remote-agents",
      extension: "roder-ext-cursor-sdk",
    },
  };
}

export class CursorCloudTaskExecutor {
  private readonly active = new ActiveRunTable();

  /**
   * Runs the execution after the ack has been written; emits output events
   * and exactly one terminal `completed`/`failed` event.
   */
  async execute(params: TaskExecuteParams, rpc: RpcWriter): Promise<void> {
    const emit = (event: TaskEvent): void => {
      rpc.notify(METHOD_TASKS_EVENT, { executionId: params.executionId, event });
    };
    const log = (chunk: string): void => {
      emit({ type: "output", stream: "log", chunk });
    };

    try {
      const wait = params.input.wait !== false;
      const input = parseCloudAgentInput(params.input);
      const sdk = await loadSdk();
      const started = await startCloudAgent(sdk, input);
      log(
        `${input.agentId ? "resumed" : "created"} cloud agent ${started.agentId} (request ${started.requestId || "pending"})`,
      );

      if (!wait) {
        // Dispatch-and-return: the cloud run continues server-side and the
        // persisted agentId can be resumed by a later task.
        emit({
          type: "completed",
          result: {
            payload: {
              agentId: started.agentId,
              requestId: started.requestId,
              status: "running",
              waited: false,
              resumed: started.resumed,
            },
          },
        });
        return;
      }

      this.active.insert(params.executionId, {
        run: started.run,
        agentId: started.agentId,
      });
      const outcome = await awaitCloudAgent(started, (status, detail) => {
        log(detail ? `status: ${status} (${detail})` : `status: ${status}`);
      });
      this.active.remove(params.executionId);

      if (outcome.status === "error") {
        emit({
          type: "failed",
          error: `cloud agent ${outcome.agentId} run ${outcome.runId} finished with status error`,
        });
        return;
      }

      emit({
        type: "completed",
        result: {
          payload: {
            agentId: outcome.agentId,
            requestId: outcome.requestId,
            runId: outcome.runId,
            status: outcome.status,
            result: outcome.resultText,
            model: outcome.model,
            durationMs: outcome.durationMs,
            branches: outcome.branches,
            prUrls: outcome.prUrls,
            waited: true,
            resumed: outcome.resumed,
          },
        },
      });
    } catch (error) {
      this.active.remove(params.executionId);
      emit({ type: "failed", error: errorMessage(error) });
    }
  }

  async cancel(executionId: string): Promise<boolean> {
    return this.active.cancel(executionId);
  }
}
