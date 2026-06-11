import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import assert from "node:assert/strict";
import {
  applyPatchTool,
  createEditWorkspace,
  createMemoryEditWorkspace,
  createRoderEditTools,
  editTool,
  loadWasmCore,
  multiEditTool,
  readFileTool,
  writeFileTool,
} from "../src/index.js";

async function tempWorkspace(prefix = "roder-edit-tools-") {
  const root = await mkdtemp(path.join(os.tmpdir(), prefix));
  return createEditWorkspace({ root });
}

test("read/edit/multi_edit/apply_patch work in a temp workspace", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "roder-edit-tools-"));
  await writeFile(path.join(root, "example.ts"), "export const one = true;\nexport const two = true;\n", "utf8");
  const workspace = await createEditWorkspace({ root });

  const read = await readFileTool(workspace, { path: "example.ts" });
  assert.equal(read.isError, false);
  assert.match(read.text, /1: export const one/);

  const edit = await editTool(workspace, { path: "example.ts", old_string: "export const one = true;", new_string: "export const one = false;" });
  assert.equal(edit.isError, false);

  const multi = await multiEditTool(workspace, { path: "example.ts", edits: [{ old_string: "export const two = true;", new_string: "export const two = false;" }] });
  assert.equal(multi.isError, false);

  const patch = await applyPatchTool(workspace, { patch: "*** Begin Patch\n*** Update File: example.ts\n@@\n-export const one = false;\n+export const one = 'patched';\n*** End Patch\n" });
  assert.equal(patch.isError, false);
  assert.match(await readFile(path.join(root, "example.ts"), "utf8"), /patched/);
});

test("profiles advertise one edit surface by default and full is explicit", async () => {
  const workspace = await tempWorkspace("roder-edit-tools-profile-");
  const oldNew = createRoderEditTools({ workspace, profile: "old-new-string" }).specs().map((spec) => spec.name);
  assert.deepEqual(oldNew, ["read_file", "write_file", "edit", "multi_edit"]);
  const patch = createRoderEditTools({ workspace, profile: "patch" }).specs().map((spec) => spec.name);
  assert.deepEqual(patch, ["read_file", "apply_patch"]);
  const full = createRoderEditTools({ workspace, profile: "full" }).specs().map((spec) => spec.name);
  assert.deepEqual(full, ["read_file", "write_file", "edit", "multi_edit", "apply_patch"]);
});

test("memory workspace supports deterministic unit tests", async () => {
  const workspace = createMemoryEditWorkspace({ "a.txt": "old\n" });
  const edit = await editTool(workspace, { path: "a.txt", old_string: "old", new_string: "new" });
  assert.equal(edit.isError, false);
  const read = await readFileTool(workspace, { path: "a.txt" });
  assert.match(read.text, /new/);
});

test("write_file returns hunks for existing files and none for new files", async () => {
  const workspace = createMemoryEditWorkspace({ "existing.txt": "old\n" });
  const existing = await writeFileTool(workspace, { path: "existing.txt", content: "new\n" });
  assert.equal(existing.isError, false);
  assert.equal((existing.data as any).hunks.length, 1);
  assert.equal((existing.data as any).hunks[0].reversePatch.includes("-new"), true);

  const created = await writeFileTool(workspace, { path: "created.txt", content: "hello\n" });
  assert.equal(created.isError, false);
  assert.deepEqual((created.data as any).hunks, []);
});

test("edit and multi_edit return structured missing and ambiguous errors without writing", async () => {
  const workspace = createMemoryEditWorkspace({ "a.txt": "same\nsame\n" });
  const ambiguous = await editTool(workspace, { path: "a.txt", old_string: "same", new_string: "changed" });
  assert.equal(ambiguous.isError, true);
  assert.equal((ambiguous.data as any).error.kind, "old_string_ambiguous");
  assert.match((await readFileTool(workspace, { path: "a.txt" })).text, /same/);

  const missing = await multiEditTool(workspace, { path: "a.txt", edits: [{ old_string: "missing", new_string: "changed" }] });
  assert.equal(missing.isError, true);
  assert.equal((missing.data as any).error.kind, "old_string_not_found");
  assert.equal((missing.data as any).error.edit, 0);
});

test("line numbers are read orientation only and not edit anchors", async () => {
  const workspace = createMemoryEditWorkspace({ "a.txt": "alpha\nbeta\n" });
  const read = await readFileTool(workspace, { path: "a.txt", start_line: 2, limit: 1 });
  assert.equal(read.text, "    2: beta");

  const edit = await editTool(workspace, { path: "a.txt", old_string: "    2: beta", new_string: "gamma" });
  if (loadWasmCore()) {
    // The Rust core strips pasted line-number prefixes and edits the
    // underlying text; the numbers themselves never act as anchors.
    assert.equal(edit.isError, false);
    assert.equal(workspace.kind === "memory" && workspace.files.get("a.txt"), "alpha\ngamma\n");
  } else {
    // The TypeScript fallback only supports exact unique matches.
    assert.equal(edit.isError, true);
    assert.equal((edit.data as any).error.kind, "old_string_not_found");
  }
});

test("workspace path traversal is rejected for fs and memory workspaces", async () => {
  const fsWorkspace = await tempWorkspace("roder-edit-tools-path-");
  await assert.rejects(
    () => createRoderEditTools({ workspace: fsWorkspace }).call({ id: "x", name: "read_file", arguments: { path: "../outside.txt" } }),
    /outside workspace/,
  );

  const memoryWorkspace = createMemoryEditWorkspace({ "safe.txt": "safe" });
  await assert.rejects(
    () => readFileTool(memoryWorkspace, { path: "../outside.txt" }),
    /outside workspace/,
  );
});

test("apply_patch supports add, delete, update and reports failures", async () => {
  const workspace = createMemoryEditWorkspace({ "update.txt": "old\n", "delete.txt": "bye\n" });
  const add = await applyPatchTool(workspace, { patch: "*** Begin Patch\n*** Add File: add.txt\n+hello\n*** End Patch\n" });
  assert.equal(add.isError, false);
  assert.match((await readFileTool(workspace, { path: "add.txt" })).text, /hello/);

  const update = await applyPatchTool(workspace, { patch: "*** Begin Patch\n*** Update File: update.txt\n@@\n-old\n+new\n*** End Patch\n" });
  assert.equal(update.isError, false);
  assert.match((await readFileTool(workspace, { path: "update.txt" })).text, /new/);

  const deletion = await applyPatchTool(workspace, { patch: "*** Begin Patch\n*** Delete File: delete.txt\n*** End Patch\n" });
  assert.equal(deletion.isError, false);
  await assert.rejects(() => readFileTool(workspace, { path: "delete.txt" }), /path does not exist/);

  const unsupported = await applyPatchTool(workspace, { patch: "--- a/update.txt\n+++ b/update.txt\n" });
  assert.equal(unsupported.isError, true);
  assert.equal((unsupported.data as any).error.kind, "unsupported_patch_format");

  const failed = await applyPatchTool(workspace, { patch: "*** Begin Patch\n*** Update File: update.txt\n@@\n-missing\n+value\n*** End Patch\n" });
  assert.equal(failed.isError, true);
  assert.equal((failed.data as any).error.kind, "apply_patch_failed");
});

test("tool adapter executes calls and reports unknown tools", async () => {
  const workspace = createMemoryEditWorkspace({ "a.txt": "old" });
  const tools = createRoderEditTools({ workspace });
  const edit = await tools.call({ id: "call-1", name: "edit", arguments: { path: "a.txt", old_string: "old", new_string: "new" } });
  assert.equal(edit.isError, false);
  assert.equal(edit.name, "edit");

  const unknown = await tools.call({ id: "call-2", name: "unknown", arguments: {} });
  assert.equal(unknown.id, "call-2");
  assert.equal(unknown.isError, true);
  assert.equal((unknown.data as any).error.kind, "unknown_tool");
});
