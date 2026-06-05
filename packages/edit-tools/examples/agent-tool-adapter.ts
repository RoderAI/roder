import { createEditWorkspace, createRoderEditTools } from "../src/index.js";

const workspace = await createEditWorkspace({ root: process.cwd() });
const tools = createRoderEditTools({ workspace, profile: "old-new-string" });
console.log(tools.specs().map((spec) => spec.name));
