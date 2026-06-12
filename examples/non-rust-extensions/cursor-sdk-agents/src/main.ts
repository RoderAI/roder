/**
 * Entry point: `node dist/src/main.js`.
 *
 * Speaks the Roder process-extension protocol (0.2.0) over stdio and
 * serves the `cursor-cloud` subagent dispatcher plus the
 * `cursor-cloud-agent` task executor backed by `@cursor/sdk`.
 * Configuration comes from explicit env vars only (the host forwards an
 * allowlist):
 *
 * - `CURSOR_API_KEY` (required for real cloud agents)
 * - `CURSOR_SDK_FAKE=1` (use the in-process fake SDK; offline tests/e2e)
 * - `RODER_EXTENSION_MANIFEST` (manifest path; default
 *   `roder-extension.toml` relative to the configured cwd)
 */

import * as fs from "node:fs";

import { CursorCloudDispatcher, dispatcherDefinitions } from "./dispatcher.js";
import {
  ALREADY_REPLIED,
  METHOD_INITIALIZE,
  METHOD_SHUTDOWN,
  METHOD_SUBAGENTS_CANCEL,
  METHOD_SUBAGENTS_DEFINITIONS,
  METHOD_SUBAGENTS_DISPATCH,
  METHOD_TASKS_CANCEL,
  METHOD_TASKS_EXECUTE,
  METHOD_TASKS_SPEC,
  PROTOCOL_VERSION,
  ShutdownRequested,
  StdioRpc,
  fnv1aChecksum,
} from "./protocol.js";
import type { SubagentDispatchParams, TaskExecuteParams } from "./protocol.js";
import { CursorCloudTaskExecutor, taskSpec } from "./tasks.js";

const EXTENSION_ID = "roder-ext-cursor-sdk";
const SERVICES = [
  { type: "subagent_dispatcher", id: "cursor-cloud" },
  { type: "task_executor", id: "cursor-cloud-agent" },
];

export function main(): Promise<void> {
  const manifestPath = process.env.RODER_EXTENSION_MANIFEST ?? "roder-extension.toml";
  const manifestChecksum = fnv1aChecksum(fs.readFileSync(manifestPath));

  const dispatcher = new CursorCloudDispatcher();
  const tasks = new CursorCloudTaskExecutor();
  const rpc = new StdioRpc();

  return rpc.run(async (method, params, writer, msgId) => {
    switch (method) {
      case METHOD_INITIALIZE:
        return {
          protocolVersion: PROTOCOL_VERSION,
          extensionId: EXTENSION_ID,
          services: SERVICES,
          manifestChecksum,
        };
      case METHOD_SUBAGENTS_DEFINITIONS:
        return { definitions: dispatcherDefinitions() };
      case METHOD_SUBAGENTS_DISPATCH: {
        // Ack first so the host's request future resolves immediately;
        // events then flow into the already-registered dispatch stream.
        const dispatch = params as unknown as SubagentDispatchParams;
        writer.reply(msgId, { dispatchId: dispatch.dispatchId });
        void dispatcher.dispatch(dispatch, writer);
        return ALREADY_REPLIED;
      }
      case METHOD_SUBAGENTS_CANCEL: {
        const dispatchId = String(params.dispatchId ?? "");
        await dispatcher.cancel(dispatchId);
        return {};
      }
      case METHOD_TASKS_SPEC:
        return { spec: taskSpec() };
      case METHOD_TASKS_EXECUTE: {
        const execute = params as unknown as TaskExecuteParams;
        writer.reply(msgId, { executionId: execute.executionId });
        void tasks.execute(execute, writer);
        return ALREADY_REPLIED;
      }
      case METHOD_TASKS_CANCEL: {
        const executionId = String(params.executionId ?? "");
        await tasks.cancel(executionId);
        return {};
      }
      case METHOD_SHUTDOWN:
        throw new ShutdownRequested();
      default:
        throw new Error(`unknown method ${method}`);
    }
  });
}

const invokedDirectly =
  process.argv[1] !== undefined && import.meta.url === `file://${process.argv[1]}`;
if (invokedDirectly) {
  main().catch((error) => {
    process.stderr.write(`fatal: ${error instanceof Error ? error.message : String(error)}\n`);
    process.exit(1);
  });
}
