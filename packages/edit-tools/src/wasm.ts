// Optional Rust/WASM core. When the generated artifact exists under
// `<package-root>/wasm/` (built by `pnpm run build:wasm` /
// scripts/build-roder-edit-wasm.sh), edit semantics run through the same
// `roder-edit-core` code that backs the Roder CLI. Without the artifact the
// package falls back to the TypeScript scaffold with matching semantics.

import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

import type { EditHunk } from "./types.js";

export interface WasmCore {
  roder_edit_tools_version(): string;
  apply_edit_json(input: string): string;
  apply_multi_edit_json(input: string): string;
  codex_patch_hunks_json(patch: string): string;
  format_line_numbered_read_json(input: string): string;
}

interface WasmEnvelope {
  ok: boolean;
  value?: {
    content: string;
    result: { path: string; replacements: number; hunks: RustEditHunk[] };
  };
  error?: {
    kind: string;
    message: string;
    data?: { error?: { kind: string; edit?: number } };
  };
}

interface RustEditHunk {
  id?: string | null;
  path: string;
  old_start: number;
  old_lines: number;
  new_start: number;
  new_lines: number;
  diff: Array<{
    kind: "added" | "removed" | "context";
    text: string;
    old_line?: number | null;
    new_line?: number | null;
  }>;
  reverse_patch?: string | null;
}

let cached: WasmCore | null | undefined;

export function loadWasmCore(): WasmCore | null {
  if (cached !== undefined) {
    return cached;
  }
  const here = path.dirname(fileURLToPath(import.meta.url));
  // src/wasm.ts -> ../wasm; dist/src/wasm.js -> ../../wasm.
  const candidates = [
    path.resolve(here, "../wasm/roder_edit_wasm.js"),
    path.resolve(here, "../../wasm/roder_edit_wasm.js"),
  ];
  const require = createRequire(import.meta.url);
  for (const candidate of candidates) {
    if (existsSync(candidate)) {
      cached = require(candidate) as WasmCore;
      return cached;
    }
  }
  cached = null;
  return null;
}

export interface WasmMultiEditOutcome {
  ok: boolean;
  content?: string;
  hunks?: EditHunk[];
  errorKind?: string;
  errorEdit?: number;
}

export function applyMultiEditViaWasm(
  core: WasmCore,
  relPath: string,
  content: string,
  edits: Array<{ old_string: string; new_string: string }>,
): WasmMultiEditOutcome {
  const envelope = JSON.parse(
    core.apply_multi_edit_json(JSON.stringify({ path: relPath, content, edits })),
  ) as WasmEnvelope;
  if (!envelope.ok || !envelope.value) {
    return {
      ok: false,
      errorKind: envelope.error?.data?.error?.kind ?? envelope.error?.kind ?? "edit_failed",
      errorEdit: envelope.error?.data?.error?.edit,
    };
  }
  return {
    ok: true,
    content: envelope.value.content,
    hunks: envelope.value.result.hunks.map(hunkFromRust),
  };
}

function hunkFromRust(hunk: RustEditHunk): EditHunk {
  return {
    id: hunk.id ?? undefined,
    path: hunk.path,
    oldStart: hunk.old_start,
    oldLines: hunk.old_lines,
    newStart: hunk.new_start,
    newLines: hunk.new_lines,
    diff: hunk.diff.map((line) => ({
      kind: line.kind,
      text: line.text,
      oldLine: line.old_line ?? undefined,
      newLine: line.new_line ?? undefined,
    })),
    reversePatch: hunk.reverse_patch ?? undefined,
  };
}
