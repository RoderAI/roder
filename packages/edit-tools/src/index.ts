import { mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import type {
  ApplyPatchArgs,
  EditArgs,
  EditHunk,
  EditWorkspace,
  EditWorkspaceOptions,
  MultiEditArgs,
  ReadFileArgs,
  ToolResult,
  WriteFileArgs,
} from "./types.js";
export type * from "./types.js";
export { createRoderEditTools } from "./tool-adapter.js";

export async function createEditWorkspace(options: EditWorkspaceOptions): Promise<EditWorkspace> {
  return { kind: "fs", root: await realpath(options.root) };
}

export function createMemoryEditWorkspace(files: Record<string, string> = {}): EditWorkspace {
  return { kind: "memory", root: "memory://workspace", files: new Map(Object.entries(files).map(([key, value]) => [normalizeMemoryPath(key), value])) };
}

export async function readFileTool(workspace: EditWorkspace, args: ReadFileArgs): Promise<ToolResult> {
  const filePath = resolveWorkspacePath(workspace, args.path);
  const content = await readWorkspaceFile(workspace, filePath);
  const startLine = Math.max(1, args.start_line ?? 1);
  const limit = Math.max(1, args.limit ?? 200);
  const text = content
    .split(/\r?\n/)
    .slice(startLine - 1, startLine - 1 + limit)
    .map((line, index) => `${String(startLine + index).padStart(5, " ")}: ${line}`)
    .join("\n");
  return ok("read_file", text, { path: displayPath(workspace, filePath), start_line: startLine, limit });
}

export async function writeFileTool(workspace: EditWorkspace, args: WriteFileArgs): Promise<ToolResult> {
  const filePath = resolveWorkspacePath(workspace, args.path);
  const previous = await readWorkspaceFileIfExists(workspace, filePath);
  await writeWorkspaceFile(workspace, filePath, args.content);
  const rel = displayPath(workspace, filePath);
  const hunks = previous === undefined ? [] : [textEditHunk(rel, previous, args.content, 0)];
  return ok("write_file", `wrote ${rel}`, { path: rel, hunks });
}

export async function editTool(workspace: EditWorkspace, args: EditArgs): Promise<ToolResult> {
  const result = await multiEditTool(workspace, {
    path: args.path,
    edits: [{ old_string: args.old_string, new_string: args.new_string }],
  });
  return { ...result, name: "edit", id: "edit" };
}

export async function multiEditTool(workspace: EditWorkspace, args: MultiEditArgs): Promise<ToolResult> {
  const filePath = resolveWorkspacePath(workspace, args.path);
  let content = await readWorkspaceFile(workspace, filePath);
  const rel = displayPath(workspace, filePath);
  const hunks: EditHunk[] = [];
  for (let index = 0; index < args.edits.length; index += 1) {
    const edit = args.edits[index];
    const first = content.indexOf(edit.old_string);
    if (first < 0) {
      return error("multi_edit", `edit ${index} old_string does not match file`, {
        error: { kind: "old_string_not_found", edit: index },
      });
    }
    if (content.indexOf(edit.old_string, first + edit.old_string.length) >= 0) {
      return error("multi_edit", `edit ${index} old_string is ambiguous`, {
        error: { kind: "old_string_ambiguous", edit: index },
      });
    }
    content = `${content.slice(0, first)}${edit.new_string}${content.slice(first + edit.old_string.length)}`;
    hunks.push(textEditHunk(rel, edit.old_string, edit.new_string, index));
  }
  await writeWorkspaceFile(workspace, filePath, content);
  return ok("multi_edit", `edited ${rel} (${args.edits.length} replacements)`, {
    path: rel,
    replacements: args.edits.length,
    hunks,
  });
}

export async function applyPatchTool(workspace: EditWorkspace, args: ApplyPatchArgs): Promise<ToolResult> {
  if (!args.patch.trim().startsWith("*** Begin Patch")) {
    return error("apply_patch", "failed to apply patch: only Codex-style patches are supported by the TypeScript scaffold", {
      error: { kind: "unsupported_patch_format" },
    });
  }
  const lines = args.patch.replace(/\r\n/g, "\n").split("\n");
  if (lines[0].trim() !== "*** Begin Patch") {
    return error("apply_patch", "failed to apply patch: missing *** Begin Patch", { error: { kind: "apply_patch_failed" } });
  }
  const hunks: EditHunk[] = [];
  for (let i = 1; i < lines.length; i += 1) {
    const line = lines[i];
    if (line === "*** End Patch") break;
    if (line.startsWith("*** Add File: ")) {
      const rel = line.slice("*** Add File: ".length).trim();
      const added: string[] = [];
      i += 1;
      while (i < lines.length && !lines[i].startsWith("*** ")) {
        if (!lines[i].startsWith("+")) return error("apply_patch", `failed to apply patch: add file ${rel} contains non-add line`, { error: { kind: "apply_patch_failed" } });
        added.push(lines[i].slice(1));
        i += 1;
      }
      i -= 1;
      const filePath = resolveWorkspacePath(workspace, rel);
      await writeWorkspaceFile(workspace, filePath, added.length === 0 ? "" : `${added.join("\n")}\n`);
      hunks.push(linesHunk(rel, [], added, hunks.length));
      continue;
    }
    if (line.startsWith("*** Delete File: ")) {
      const rel = line.slice("*** Delete File: ".length).trim();
      await deleteWorkspaceFile(workspace, resolveWorkspacePath(workspace, rel));
      hunks.push(linesHunk(rel, [], [], hunks.length));
      continue;
    }
    if (line.startsWith("*** Update File: ")) {
      const rel = line.slice("*** Update File: ".length).trim();
      const filePath = resolveWorkspacePath(workspace, rel);
      let content = await readWorkspaceFile(workspace, filePath);
      i += 1;
      while (i < lines.length && !lines[i].startsWith("*** ")) {
        if (!lines[i].startsWith("@@")) return error("apply_patch", `failed to apply patch: ${rel}: expected hunk header`, { error: { kind: "apply_patch_failed" } });
        const oldLines: string[] = [];
        const newLines: string[] = [];
        i += 1;
        while (i < lines.length && !lines[i].startsWith("@@") && !lines[i].startsWith("*** ")) {
          const hunkLine = lines[i];
          if (hunkLine === "*** End of File") {
            i += 1;
            continue;
          }
          const body = hunkLine.slice(1);
          if (hunkLine.startsWith(" ")) {
            oldLines.push(body);
            newLines.push(body);
          } else if (hunkLine.startsWith("-")) {
            oldLines.push(body);
          } else if (hunkLine.startsWith("+")) {
            newLines.push(body);
          } else {
            return error("apply_patch", `failed to apply patch: ${rel}: invalid hunk line`, { error: { kind: "apply_patch_failed" } });
          }
          i += 1;
        }
        i -= 1;
        const oldText = oldLines.join("\n");
        const newText = newLines.join("\n");
        const position = content.indexOf(oldText);
        if (position < 0) return error("apply_patch", `failed to apply patch: expected hunk not found in ${rel}`, { error: { kind: "apply_patch_failed" } });
        content = `${content.slice(0, position)}${newText}${content.slice(position + oldText.length)}`;
        hunks.push(linesHunk(rel, oldLines, newLines, hunks.length));
        i += 1;
      }
      i -= 1;
      await writeWorkspaceFile(workspace, filePath, content);
      continue;
    }
    if (line.trim() !== "") return error("apply_patch", `failed to apply patch: unexpected line ${JSON.stringify(line)}`, { error: { kind: "apply_patch_failed" } });
  }
  return ok("apply_patch", "Success. Applied patch", { hunks });
}

async function readWorkspaceFile(workspace: EditWorkspace, filePath: string): Promise<string> {
  if (workspace.kind === "memory") {
    const rel = normalizeMemoryPath(displayPath(workspace, filePath));
    const value = workspace.files.get(rel);
    if (value === undefined) throw new Error(`path does not exist: ${rel}`);
    return value;
  }
  return readFile(filePath, "utf8");
}

async function readWorkspaceFileIfExists(workspace: EditWorkspace, filePath: string): Promise<string | undefined> {
  try {
    return await readWorkspaceFile(workspace, filePath);
  } catch {
    return undefined;
  }
}

async function writeWorkspaceFile(workspace: EditWorkspace, filePath: string, content: string): Promise<void> {
  if (workspace.kind === "memory") {
    workspace.files.set(normalizeMemoryPath(displayPath(workspace, filePath)), content);
    return;
  }
  await mkdir(path.dirname(filePath), { recursive: true });
  await writeFile(filePath, content, "utf8");
}

async function deleteWorkspaceFile(workspace: EditWorkspace, filePath: string): Promise<void> {
  if (workspace.kind === "memory") {
    workspace.files.delete(normalizeMemoryPath(displayPath(workspace, filePath)));
    return;
  }
  await rm(filePath);
}

function normalizeMemoryPath(input: string): string {
  return input.replaceAll("\\", "/").replace(/^\/+/, "");
}

function resolveWorkspacePath(workspace: EditWorkspace, input: string): string {
  if (input.trim() === "") throw new Error("path is required");
  if (workspace.kind === "memory") {
    const normalized = path.posix.normalize(normalizeMemoryPath(input));
    if (normalized.startsWith("..") || path.posix.isAbsolute(normalized)) {
      throw new Error(`path ${input} is outside workspace ${workspace.root}`);
    }
    return `${workspace.root}/${normalized}`;
  }
  const candidate = path.resolve(workspace.root, input);
  const relative = path.relative(workspace.root, candidate);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error(`path ${candidate} is outside workspace ${workspace.root}`);
  }
  return candidate;
}

function displayPath(workspace: EditWorkspace, filePath: string): string {
  if (workspace.kind === "memory") {
    return normalizeMemoryPath(filePath.slice(`${workspace.root}/`.length));
  }
  return path.relative(workspace.root, filePath).replaceAll(path.sep, "/");
}

function ok<TData>(name: string, text: string, data: TData): ToolResult<TData> {
  return { id: name, name, text, data, isError: false };
}

function error<TData>(name: string, text: string, data: TData): ToolResult<TData> {
  return { id: name, name, text, data, isError: true };
}

function textEditHunk(rel: string, oldText: string, newText: string, index: number): EditHunk {
  return linesHunk(rel, oldText.split(/\r?\n/).filter((_, i, arr) => i < arr.length - 1 || arr[i] !== ""), newText.split(/\r?\n/).filter((_, i, arr) => i < arr.length - 1 || arr[i] !== ""), index);
}

function linesHunk(rel: string, oldLines: string[], newLines: string[], index: number): EditHunk {
  return {
    id: `hunk-${index + 1}`,
    path: rel,
    oldStart: 1,
    oldLines: oldLines.length,
    newStart: 1,
    newLines: newLines.length,
    diff: [
      ...oldLines.map((text, lineIndex) => ({ kind: "removed" as const, text, oldLine: lineIndex + 1 })),
      ...newLines.map((text, lineIndex) => ({ kind: "added" as const, text, newLine: lineIndex + 1 })),
    ],
    reversePatch: reversePatch(rel, oldLines, newLines),
  };
}

function reversePatch(rel: string, oldLines: string[], newLines: string[]): string {
  return `*** Begin Patch\n*** Update File: ${rel}\n@@\n${newLines.map((line) => `-${line}`).join("\n")}${newLines.length > 0 ? "\n" : ""}${oldLines.map((line) => `+${line}`).join("\n")}${oldLines.length > 0 ? "\n" : ""}*** End Patch\n`;
}
