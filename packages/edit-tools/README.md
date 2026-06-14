# @roderai/edit-tools

Roder-grade file editing tools for JavaScript agent runtimes.

Roder website: https://roder.sh

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

## Publishing

Publish from this directory after the release version is already reflected in
`package.json`, `CHANGELOG.md`, and the generated `dist/` files:

```sh
pnpm pack --dry-run
npm publish --access public --registry=https://registry.npmjs.org/
```

The package is published as `@roderai/edit-tools`; keep `README.md` in the
`package.json` `files` list so npm shows this page on the registry.
