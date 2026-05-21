# Roder SDK Codegen

These scripts generate SDK type inputs from `schemas/app-server/roder-app-server.v1.json`.

Run:

```sh
node sdk/codegen/generate-typescript.mjs
node sdk/codegen/generate-python.mjs
```

Check mode verifies generated files are current without rewriting them:

```sh
node sdk/codegen/generate-typescript.mjs --check
node sdk/codegen/generate-python.mjs --check
```

Generated files are not hand edited. Update the Rust app-server manifest first, export the checked manifest, then rerun codegen.
