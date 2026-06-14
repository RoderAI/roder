# @roderai/sdk

TypeScript SDK for the Roder app-server JSON-RPC API.

```ts
import { RoderAgent } from "@roderai/sdk";
```

Normal tests use in-memory fake transports. Live local and remote smoke checks are opt-in with `RODER_SDK_LIVE=1`.

For process-based automation, spawn `roder exec --json` and consume one JSON
event per stdout line:

```sh
printf 'Reply with exactly: ok\n' | roder exec --json --profile eval --mode bypass -
```

See `docs/roder-exec.md` for the JSONL event contract.

Before packing:

```sh
pnpm run typecheck
pnpm test -- fixtures
pnpm pack --dry-run
```

## Publishing

Publish from this directory after the release version is already reflected in
`package.json`, `CHANGELOG.md`, and the generated `dist/` files:

```sh
pnpm pack --dry-run
npm publish --access public --registry=https://registry.npmjs.org/
```

The package is published as `@roderai/sdk`; keep `README.md` in the
`package.json` `files` list so npm shows this page on the registry.
