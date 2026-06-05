import { createEditWorkspace, editTool } from "../src/index.js";

const workspace = await createEditWorkspace({ root: process.cwd() });
const result = await editTool(workspace, {
  path: "src/example.ts",
  old_string: "export const enabled = true;",
  new_string: "export const enabled = false;",
});
console.log(result);
