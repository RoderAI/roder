/**
 * Shared remote-agent driving logic for the dispatcher and task executor:
 * strict input validation, create-or-resume through the SDK seam, stream
 * consumption with redacted progress callbacks, cancellation, and result
 * shaping.
 */

import { redactSecrets } from "./protocol.js";
import type { SdkModule, SdkRun, SdkRunResult } from "./sdk.js";

/** Validated input for one remote Cursor cloud agent operation. */
export interface CloudAgentInput {
  prompt: string;
  model?: string;
  /** Resume an existing cloud agent instead of creating one. */
  agentId?: string;
  repoUrl?: string;
  startingRef?: string;
  autoCreatePr: boolean;
}

const INPUT_KEYS = new Set([
  "prompt",
  "model",
  "agentId",
  "repoUrl",
  "startingRef",
  "autoCreatePr",
  "wait",
]);

/**
 * Validates dispatch input strictly and early, before any SDK call.
 * `extra.prompt`/`extra.model` (from the canonical SubagentRequest) take
 * precedence over keys embedded in the structured input object.
 */
export function parseCloudAgentInput(
  raw: Record<string, unknown>,
  extra?: { prompt?: string; model?: string },
): CloudAgentInput {
  for (const key of Object.keys(raw)) {
    if (!INPUT_KEYS.has(key)) {
      throw new Error(
        `unsupported input key ${JSON.stringify(key)}; supported keys: ${[...INPUT_KEYS].join(", ")}`,
      );
    }
  }
  const prompt = extra?.prompt ?? stringField(raw, "prompt");
  if (!prompt || prompt.trim().length === 0) {
    throw new Error("prompt is required and must be a non-empty string");
  }
  const model = extra?.model ?? stringField(raw, "model");
  const agentId = stringField(raw, "agentId");
  const repoUrl = stringField(raw, "repoUrl");
  const startingRef = stringField(raw, "startingRef");
  const autoCreatePr = raw.autoCreatePr;
  if (autoCreatePr !== undefined && typeof autoCreatePr !== "boolean") {
    throw new Error("autoCreatePr must be a boolean");
  }

  if (agentId) {
    if (!agentId.startsWith("bc-")) {
      throw new Error("agentId must be a cloud agent id (bc- prefix)");
    }
    if (repoUrl || startingRef || autoCreatePr !== undefined) {
      throw new Error(
        "repoUrl, startingRef, and autoCreatePr only apply when creating an agent; omit them when resuming by agentId",
      );
    }
  } else {
    if (!repoUrl) {
      throw new Error("repoUrl is required when creating a cloud agent");
    }
    if (!/^https:\/\/\S+\/\S+/.test(repoUrl)) {
      throw new Error("repoUrl must be an https repository URL");
    }
  }

  return {
    prompt,
    model: model || undefined,
    agentId: agentId || undefined,
    repoUrl: repoUrl || undefined,
    startingRef: startingRef || undefined,
    autoCreatePr: autoCreatePr === true,
  };
}

function stringField(raw: Record<string, unknown>, key: string): string | undefined {
  const value = raw[key];
  if (value === undefined || value === null) {
    return undefined;
  }
  if (typeof value !== "string") {
    throw new Error(`${key} must be a string`);
  }
  return value;
}

export interface CloudAgentOutcome {
  agentId: string;
  requestId: string;
  runId: string;
  status: SdkRunResult["status"];
  resultText: string;
  model: string | null;
  durationMs: number | null;
  branches: Array<{ repoUrl: string; branch?: string; prUrl?: string }>;
  prUrls: string[];
  resumed: boolean;
}

export interface ActiveRun {
  run: SdkRun;
  agentId: string;
}

/** Live runs keyed by host-chosen dispatch/execution id, for cancellation. */
export class ActiveRunTable {
  private readonly runs = new Map<string, ActiveRun>();

  insert(id: string, active: ActiveRun): void {
    this.runs.set(id, active);
  }

  remove(id: string): void {
    this.runs.delete(id);
  }

  async cancel(id: string): Promise<boolean> {
    const active = this.runs.get(id);
    if (!active) {
      return false;
    }
    this.runs.delete(id);
    await active.run.cancel();
    return true;
  }
}

export interface StartedCloudAgent {
  agentId: string;
  requestId: string;
  run: SdkRun;
  resumed: boolean;
}

/**
 * Creates or resumes the cloud agent and submits the prompt. The API key
 * comes from `CURSOR_API_KEY` only (forwarded by the host env allowlist)
 * and never appears in errors or results.
 */
export async function startCloudAgent(
  sdk: SdkModule,
  input: CloudAgentInput,
): Promise<StartedCloudAgent> {
  const apiKey = process.env.CURSOR_API_KEY;
  if (!apiKey || apiKey.trim().length === 0) {
    throw new Error(
      "CURSOR_API_KEY is not configured; add it to the process extension env allowlist",
    );
  }

  const resumed = Boolean(input.agentId);
  const agent = input.agentId
    ? await sdk.Agent.resume(input.agentId, { apiKey })
    : await sdk.Agent.create({
        apiKey,
        ...(input.model ? { model: { id: input.model } } : {}),
        cloud: {
          repos: [
            {
              url: input.repoUrl,
              ...(input.startingRef ? { startingRef: input.startingRef } : {}),
            },
          ],
          autoCreatePR: input.autoCreatePr,
        },
      });

  const run = await agent.send(input.prompt);
  return {
    agentId: agent.agentId,
    requestId: run.requestId ?? "",
    run,
    resumed,
  };
}

/**
 * Consumes the run stream, reporting redacted progress lines, then awaits
 * the final result. `tool_call` payloads are documented-unstable upstream:
 * only the stable envelope (name, status) is forwarded; args/results are
 * dropped.
 */
export async function awaitCloudAgent(
  started: StartedCloudAgent,
  onProgress: (status: string, detail?: string) => void,
): Promise<CloudAgentOutcome> {
  for await (const event of started.run.stream()) {
    switch (event.type) {
      case "status": {
        const status = typeof event.status === "string" ? event.status : "UNKNOWN";
        const detail =
          typeof event.message === "string" ? redactSecrets(event.message) : undefined;
        onProgress(status, detail);
        break;
      }
      case "task": {
        if (typeof event.text === "string" && event.text.length > 0) {
          onProgress("TASK", redactSecrets(event.text));
        }
        break;
      }
      case "tool_call": {
        const name = typeof event.name === "string" ? event.name : "tool";
        const status = typeof event.status === "string" ? event.status : "running";
        onProgress("TOOL", `${name}: ${status}`);
        break;
      }
      default:
        break;
    }
  }

  const result = await started.run.wait();
  const branches = result.git?.branches ?? [];
  return {
    agentId: started.agentId,
    requestId: result.requestId ?? started.requestId,
    runId: result.id,
    status: result.status,
    resultText: redactSecrets(result.result ?? ""),
    model: result.model?.id ?? null,
    durationMs: result.durationMs ?? null,
    branches,
    prUrls: branches.flatMap((branch) => (branch.prUrl ? [branch.prUrl] : [])),
    resumed: started.resumed,
  };
}
