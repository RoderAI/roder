# Roder Python Chat-Completions Provider (POC)

A process-hosted Roder extension written in Python (stdlib only). It
registers an OpenAI-compatible chat-completions inference provider and an
event sink through the standard extension registry, speaking the Roder
process-extension protocol over stdio. See
`docs/roder-process-extensions.md` for the protocol and security model.

## Configure

```toml
[[process_extensions]]
id = "python-chat-completions"
enabled = true
manifest = "examples/non-rust-extensions/python-chat-completions/roder-extension.toml"
command = "python3"
args = ["-m", "roder_python_chat_provider"]
cwd = "examples/non-rust-extensions/python-chat-completions"
env = { PYTHONUNBUFFERED = "1", PYTHONPATH = "src", PY_CHAT_COMPLETIONS_API_KEY = "sk-..." }
event_filter = { kinds = ["turn."] }
```

Environment (forwarded explicitly through `env` — the host never passes its
full environment):

- `PY_CHAT_COMPLETIONS_API_KEY` — bearer token (required for real turns)
- `PY_CHAT_COMPLETIONS_BASE_URL` — default `https://api.openai.com/v1`
- `PY_CHAT_COMPLETIONS_MODEL` — optional model override
- `RODER_EXTENSION_MANIFEST` — manifest path override (default
  `roder-extension.toml` relative to `cwd`)

The provider records received Roder event kinds (names and counts only) and
reports them in `ProviderMetadata`; prompts, keys, and headers never appear
in events.

## Test (offline)

```sh
python3 -m unittest discover -s tests
```

Uses a local fake SSE server; no network or credentials. `uv run pytest`
also works if you prefer pytest (`pip install -e .[dev]`).

The Rust end-to-end proof that this package serves a full turn through the
public app-server surfaces:

```sh
cargo test -p roder-app-server --features e2e-tests --test process_extension_python_provider
```

## Live smoke (opt-in)

```sh
RODER_PROCESS_EXT_LIVE=1 \
PY_CHAT_COMPLETIONS_API_KEY="$OPENAI_API_KEY" \
PY_CHAT_COMPLETIONS_BASE_URL="https://api.openai.com/v1" \
PY_CHAT_COMPLETIONS_MODEL="gpt-5.5" \
cargo test -p roder-app-server --features e2e-tests \
  --test process_extension_python_provider -- --ignored --nocapture
```
