// Generated-WASM core tests. These require the artifact under
// `<package-root>/wasm/` (build with `pnpm run build:wasm`); without it the
// suite reports the missing artifact and skips so plain checkouts stay green.

import assert from "node:assert/strict";
import test from "node:test";

import {
  createMemoryEditWorkspace,
  editTool,
  loadWasmCore,
  multiEditTool,
} from "../src/index.js";

const wasm = loadWasmCore();

test("wasm core artifact loads and reports its version", (t) => {
  if (!wasm) {
    t.skip("wasm artifact not built (run pnpm run build:wasm)");
    return;
  }
  assert.match(wasm.roder_edit_tools_version(), /^\d+\.\d+\.\d+$/);
});

test("multi_edit runs through the Rust core when the artifact is present", async (t) => {
  if (!wasm) {
    t.skip("wasm artifact not built (run pnpm run build:wasm)");
    return;
  }
  const workspace = createMemoryEditWorkspace({ "src/a.txt": "alpha\nbeta\n" });
  const result = await multiEditTool(workspace, {
    path: "src/a.txt",
    edits: [
      { old_string: "alpha", new_string: "ALPHA" },
      { old_string: "beta", new_string: "BETA" },
    ],
  });

  assert.equal(result.isError, false);
  assert.equal((result.data as any).core, "wasm");
  assert.equal((result.data as any).hunks.length, 2);
  assert.equal((result.data as any).hunks[0].path, "src/a.txt");
  assert.ok((result.data as any).hunks[0].reversePatch.includes("-ALPHA"));
  assert.equal(workspace.kind === "memory" && workspace.files.get("src/a.txt"), "ALPHA\nBETA\n");
});

test("rust-core ambiguity and not-found semantics surface through wasm", async (t) => {
  if (!wasm) {
    t.skip("wasm artifact not built (run pnpm run build:wasm)");
    return;
  }
  const workspace = createMemoryEditWorkspace({ "dup.txt": "same\nsame\n" });
  const ambiguous = await editTool(workspace, {
    path: "dup.txt",
    old_string: "same",
    new_string: "different",
  });
  assert.equal(ambiguous.isError, true);
  assert.equal((ambiguous.data as any).error.kind, "old_string_ambiguous");

  const missing = await editTool(workspace, {
    path: "dup.txt",
    old_string: "not present",
    new_string: "x",
  });
  assert.equal(missing.isError, true);
  assert.equal((missing.data as any).error.kind, "old_string_not_found");
});
