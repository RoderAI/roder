# @roder/sdk

TypeScript SDK for the Roder app-server JSON-RPC API.

```ts
import { RoderAgent } from "@roder/sdk";
```

Normal tests use in-memory fake transports. Live local and remote smoke checks are opt-in with `RODER_SDK_LIVE=1`.

Before packing:

```sh
pnpm run typecheck
pnpm test -- fixtures
pnpm pack --dry-run
```
