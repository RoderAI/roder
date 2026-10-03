# @roderai/sdk

TypeScript SDK for the Roder app-server JSON-RPC API.

Roder website: https://roder.sh

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

### Tools in the user's browser

For a hosted thread, bind `ExternalToolExecutor` before `turn/start`, then feed it
live notifications from your existing client loop. The gateway gives one
authenticated connection an opaque lease; a second tab receives `executor_busy`.
Use explicit `takeover: true` only for a user-requested handoff. Call `stop()` on
transport loss or page teardown, and `close()` when deliberately unbinding.
Callbacks receive an `AbortSignal` and must check it before committing effects.
Never replay transcript items as execution requests. `read(requestId)` returns
pending/terminal metadata, without tool inputs or outputs, for recovery.

`RoderAgent` binds automatically for `remote` external-tool callbacks. With a
custom transport connected to the hosted gateway, set
`externalToolExecution: "hosted"`. Local app-server callbacks use local execution.
The helper starts no model loop and contains no application or DOM semantics.

Browser bundlers select a remote-only entry automatically. You can also import
`@roderai/sdk/browser` explicitly; it exports the RPC client, WebSocket transport,
normalized events, runs, and external-tool executor without Node process imports.
Use the default Node entry for `RoderAgent` and local process execution.
