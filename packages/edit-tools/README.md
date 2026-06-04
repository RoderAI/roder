# @roderai/edit-tools

Roder-grade file editing tools for JavaScript agent runtimes.

Use this package when you want Roder's edit semantics without adopting the full Roder app-server, TUI, or agent loop. `@roderai/sdk` controls Roder app-server sessions; `@roderai/edit-tools` only edits files in a workspace you provide.

```ts
import { createEditWorkspace, editTool } from "@roderai/edit-tools";

const workspace = await createEditWorkspace({ root: process.cwd() });
await editTool(workspace, {
  path: "src/example.ts",
  old_string: "return true;",
  new_string: "return false;",
});
```

## Edit surfaces

- `old-new-string`: advertises `read_file`, `write_file`, `edit`, and `multi_edit`.
- `patch`: advertises `read_file` and `apply_patch`.
- `full`: explicit experimental profile for hosts that intentionally want both edit surfaces.

Line numbers in `read_file` output are for orientation only. Do not include them in `old_string` or `new_string`.

## Security

Paths are resolved under the configured workspace root. Traversal outside the root is rejected. The package does not start Roder, shell out to providers, run the TUI, or publish anything to npm during tests.
