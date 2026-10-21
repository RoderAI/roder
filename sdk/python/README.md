# roder-sdk

Python SDK for the Roder app-server JSON-RPC API.

Roder website: https://roder.sh

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

## Publishing

Publish from this directory after the release version is already reflected in
`pyproject.toml`, `CHANGELOG.md`, and `uv.lock`:

```sh
uv build
uv publish
```

The package is published as `roder-sdk`; keep `readme = "README.md"` in
`pyproject.toml` so PyPI shows this page on the project page.

## Inference service tiers

Set a persistent thread default at creation, override it for one turn, or change
it on an existing thread. Turn overrides take precedence over the thread tier
and provider config; thread defaults survive server restarts.

```python
async with await RoderAgent.create(
    local={"command": "roder"}, cwd="/workspace", service_tier="ultrafast",
) as agent:
    await agent.send("Inspect the repo")
    await agent.send("Use standard processing for this turn", service_tier="default")
    await agent.set_service_tier("ultrafast")
    await agent.set_service_tier(None)  # Restore the provider default.
```
