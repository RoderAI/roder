export type EditSurfaceProfile = "old-new-string" | "patch" | "full";

export interface EditWorkspaceOptions {
  root: string;
}

export interface FileSystemEditWorkspace {
  kind: "fs";
  root: string;
}

export interface MemoryEditWorkspace {
  kind: "memory";
  root: string;
  files: Map<string, string>;
}

export type EditWorkspace = FileSystemEditWorkspace | MemoryEditWorkspace;

export interface ToolSpec {
  name: string;
  description: string;
  parameters: unknown;
}

export interface ToolCall {
  id: string;
  name: string;
  arguments: unknown;
}

export interface ToolResult<TData = unknown> {
  id: string;
  name: string;
  text: string;
  data: TData;
  isError: boolean;
}

export interface ReadFileArgs {
  path: string;
  start_line?: number;
  limit?: number;
}

export interface WriteFileArgs {
  path: string;
  content: string;
}

export interface EditArgs {
  path: string;
  old_string: string;
  new_string: string;
}

export interface MultiEditArgs {
  path: string;
  edits: Array<{ old_string: string; new_string: string }>;
}

export interface ApplyPatchArgs {
  patch: string;
}

export interface EditHunk {
  id?: string;
  path: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  diff: Array<{
    kind: "added" | "removed" | "context";
    text: string;
    oldLine?: number;
    newLine?: number;
  }>;
  reversePatch?: string;
}
