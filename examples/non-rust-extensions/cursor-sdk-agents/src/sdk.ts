/**
 * Thin seam over `@cursor/sdk` so the extension logic and its tests never
 * import the real SDK directly.
 *
 * - Production: `loadSdk()` dynamically imports `@cursor/sdk` (pinned in
 *   package.json; see README for the recorded version).
 * - Offline tests and the Rust e2e: `CURSOR_SDK_FAKE=1` selects the
 *   in-process fake from `fake.js`, or tests inject a module with
 *   `setSdkModule()`.
 *
 * The types below are the subset of the documented SDK surface this
 * extension relies on. `tool_call` args/results are deliberately typed as
 * `unknown` — Cursor documents them as unstable; only the envelope is
 * stable.
 */

export interface SdkModelSelection {
  id: string;
  [key: string]: unknown;
}

export interface SdkRunGitInfo {
  branches: Array<{ repoUrl: string; branch?: string; prUrl?: string }>;
}

export interface SdkRunResult {
  id: string;
  requestId?: string;
  status: "finished" | "error" | "cancelled";
  result?: string;
  model?: SdkModelSelection;
  durationMs?: number;
  git?: SdkRunGitInfo;
}

/** Normalized stream event; discriminate on `type`. */
export interface SdkMessage {
  type: string;
  status?: string;
  message?: unknown;
  text?: string;
  name?: string;
  call_id?: string;
  [key: string]: unknown;
}

export interface SdkRun {
  readonly requestId?: string;
  stream(): AsyncGenerator<SdkMessage, void, void>;
  wait(): Promise<SdkRunResult>;
  cancel(): Promise<void>;
}

export interface SdkAgent {
  readonly agentId: string;
  send(prompt: string, options?: Record<string, unknown>): Promise<SdkRun>;
  close?(): void;
}

export interface SdkModel {
  id: string;
  [key: string]: unknown;
}

export interface SdkModule {
  Agent: {
    create(options: Record<string, unknown>): Promise<SdkAgent>;
    resume(agentId: string, options?: Record<string, unknown>): Promise<SdkAgent>;
  };
  Cursor?: {
    models: {
      list(options?: Record<string, unknown>): Promise<SdkModel[]>;
    };
  };
}

let injected: SdkModule | null = null;

/** Test seam: inject a fake module instead of importing `@cursor/sdk`. */
export function setSdkModule(module: SdkModule | null): void {
  injected = module;
}

export async function loadSdk(): Promise<SdkModule> {
  if (injected) {
    return injected;
  }
  if (process.env.CURSOR_SDK_FAKE === "1") {
    const fake = await import("./fake.js");
    return fake.createFakeSdkModule();
  }
  return (await import("@cursor/sdk")) as unknown as SdkModule;
}
