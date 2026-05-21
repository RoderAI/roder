# roder-sdk

Python SDK for the Roder app-server JSON-RPC API.

```py
from roder_sdk import RoderAgent
```

Normal tests use in-memory fake transports. Live local and remote smoke checks are opt-in with `RODER_SDK_LIVE=1`.

Before building:

```sh
uv run pytest tests
uv run pyright src
```
