# roder-sdk

Python SDK for the Roder app-server JSON-RPC API.

```py
from roder_sdk import RoderAgent
```

Normal tests use in-memory fake transports. Live local and remote smoke checks are opt-in with `RODER_SDK_LIVE=1`.

For process-based automation, spawn `roder exec --json` and consume one JSON
event per stdout line:

```sh
printf 'Reply with exactly: ok\n' | roder exec --json --profile eval --mode bypass -
```

See `docs/roder-exec.md` for the JSONL event contract.

Before building:

```sh
uv run pytest tests
uv run pyright src
```
