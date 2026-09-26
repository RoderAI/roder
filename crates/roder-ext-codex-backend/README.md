# roder-ext-codex-backend

`roder-ext-codex-backend` is the Codex agent backend for [Roder](https://roder.sh).

## What It Does

Registers an `AgentBackend` named `codex`, which runs a private Codex
app-server over its stable stdio JSONL transport. Where Roder's own runtime
drives inference and tools itself, this backend delegates a turn to Codex and
projects its events back into Roder's event stream, so a session can be served
by Codex while keeping Roder's session, transcript and client surfaces.

## Using It

The extension is installed through Roder's extension registry:

```rust
use roder_ext_codex_backend::CodexBackendExtension;

registry.install(CodexBackendExtension)?;
```

The backend advertises Codex's own model catalog and runs Codex's tool set;
Roder tool providers are not advertised through Codex dynamic tools.
