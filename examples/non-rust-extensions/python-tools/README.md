# Roder Python Tools Extension (example)

A process-hosted Roder extension written in Python (stdlib only, 3.11+ for
`tomllib`) that contributes a model-callable tool. The manifest declares the
tool schema statically under a `tool_provider` service, so Roder registers
the tool without spawning the child; the child only starts on the first
`tools/call`. See `docs/roder-process-extensions.md` for the protocol
(version `0.2.0`) and security model.

The manifest also carries a `[launch]` section, so the package layer can
start the child without a separate config entry. A hand-written
`[[process_extensions]]` config entry works too:

```toml
[[process_extensions]]
id = "python-tools"
manifest = "examples/non-rust-extensions/python-tools/roder-extension.toml"
command = "python3"
args = ["main.py"]
cwd = "examples/non-rust-extensions/python-tools"
```

Environment (forwarded explicitly through `env` — the host never passes its
full environment):

- `RODER_EXTENSION_MANIFEST` — manifest path override (default:
  `roder-extension.toml` next to `main.py`)

## Test (offline)

```sh
python3 -m unittest discover -s tests
```

The Rust host end-to-end coverage for process-hosted tools lives in
`crates/roder-ext-process-host/tests/host.rs`.
