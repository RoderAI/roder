import { applyPatchTool, editTool, multiEditTool, readFileTool, writeFileTool } from "./index.js";
import type { EditSurfaceProfile, EditWorkspace, ToolCall, ToolResult, ToolSpec } from "./types.js";

export function createRoderEditTools(options: { workspace: EditWorkspace; profile?: EditSurfaceProfile }) {
  const profile = options.profile ?? "old-new-string";
  const specs = toolSpecs().filter((spec) => toolNamesForProfile(profile).includes(spec.name));
  return {
    specs: () => specs,
    async call(call: ToolCall): Promise<ToolResult> {
      switch (call.name) {
        case "read_file":
          return readFileTool(options.workspace, call.arguments as never);
        case "write_file":
          return writeFileTool(options.workspace, call.arguments as never);
        case "edit":
          return editTool(options.workspace, call.arguments as never);
        case "multi_edit":
          return multiEditTool(options.workspace, call.arguments as never);
        case "apply_patch":
          return applyPatchTool(options.workspace, call.arguments as never);
        default:
          return { id: call.id, name: call.name, text: `unknown tool: ${call.name}`, data: { error: { kind: "unknown_tool" } }, isError: true };
      }
    },
  };
}

function toolNamesForProfile(profile: EditSurfaceProfile): string[] {
  switch (profile) {
    case "old-new-string":
      return ["read_file", "write_file", "edit", "multi_edit"];
    case "patch":
      return ["read_file", "apply_patch"];
    case "full":
      return ["read_file", "write_file", "edit", "multi_edit", "apply_patch"];
  }
}

function toolSpecs(): ToolSpec[] {
  return [
    { name: "read_file", description: "Read a UTF-8 text file with line numbers for orientation.", parameters: { type: "object", required: ["path"], properties: { path: { type: "string" }, start_line: { type: "integer", minimum: 1 }, limit: { type: "integer", minimum: 1 } }, additionalProperties: false } },
    { name: "write_file", description: "Write a UTF-8 text file.", parameters: { type: "object", required: ["path", "content"], properties: { path: { type: "string" }, content: { type: "string" } }, additionalProperties: false } },
    { name: "edit", description: "Apply one exact old_string/new_string replacement.", parameters: { type: "object", required: ["path", "old_string", "new_string"], properties: { path: { type: "string" }, old_string: { type: "string" }, new_string: { type: "string" } }, additionalProperties: false } },
    { name: "multi_edit", description: "Apply ordered exact old_string/new_string replacements.", parameters: { type: "object", required: ["path", "edits"], properties: { path: { type: "string" }, edits: { type: "array", items: { type: "object", required: ["old_string", "new_string"], properties: { old_string: { type: "string" }, new_string: { type: "string" } }, additionalProperties: false } } }, additionalProperties: false } },
    { name: "apply_patch", description: "Apply a Codex-style patch in the workspace.", parameters: { type: "object", required: ["patch"], properties: { patch: { type: "string" } }, additionalProperties: false } },
  ];
}
