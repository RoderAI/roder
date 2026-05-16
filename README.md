# Roder

**Rust Coder.** A Rust-native, extension-first agent harness for coding agents, research systems, reinforcement-learning environments, and AI-native developer tools.

> Roder is the core harness. Everything else is a distribution, extension, provider, policy, interface, or experiment.

Roder is not just another coding agent. It is the substrate underneath them — a stable, strongly typed Rust runtime that handles inference orchestration, tool execution, capability-scoped filesystem and process access, context assembly, session persistence, checkpointing, policy enforcement, event streaming, and replay, with extension points designed so that new model APIs, storage systems, context engines, sandbox backends, UIs, and training environments can be added without forking the core.

The full mission, design philosophy, and architectural commitments live in [`WHITEPAPER.md`](./WHITEPAPER.md). The phased implementation plan lives in [`roadmap/`](./roadmap/), entry point [`roadmap/00-feature-inventory-and-sequencing.md`](./roadmap/00-feature-inventory-and-sequencing.md).

---

## What Roder represents

Most agent systems today are rebuilt repeatedly around the same primitives — model invocation, tool execution, sandboxing, context, session persistence, policy, event streaming, replay — usually in Python or Node.js. That works for fast iteration but fragments the ecosystem: every team re-solves the same harness problems in incompatible ways, and labs end up forking upstream because the architecture cannot express the modifications they need.

Roder's bet is that this is unnecessary. The AI ecosystem should not need to rewrite the same agent harness every time a new model, product, research direction, or interaction paradigm appears.

Roder aims to be that foundation, once, with:

- **A stable core that owns invariants.** Lifecycle ordering, cancellation, event ordering, permission enforcement, tool routing, and capability checks live in `roder-core` and are not extensible — extensions provide behavior, but they cannot corrupt the runtime.
- **A native extension kernel.** Inference engines, wire dialects, context providers, context planners, session stores, checkpoint stores, memory backends, policy contributors, sandbox backends, event sinks, and tool contributors all install through a single `RoderExtension` trait against `roder-api`.
- **Canonical internal representations.** Conversations, turns, messages, tool calls, inference events, file changes, context blocks, sessions, checkpoints, and policy decisions are typed Roder concepts. Provider extensions translate to and from these canonical types; the core never sees Responses, Chat Completions, or Anthropic Messages wire formats.
- **Capability-based access.** Tools and extensions receive `ScopedFilesystem`, `ScopedProcessRunner`, `ScopedNetwork`, `ScopedSecrets`, `ApprovalClient`, and `EventSink` handles. There is no ambient `std::fs` or `std::process::Command` access.
- **Event-sourced execution.** Every meaningful runtime transition is a typed event on the canonical bus. This makes Roder replayable, auditable, resumable, and usable as an RL trajectory substrate without bespoke instrumentation.
- **App-server-first control plane.** The TUI is a client of the embedded local app server. IDE plugins, web UIs, headless CI runners, RL harnesses, and IDE extensions all speak the same control-plane protocol.

The whitepaper lays out the long-term aspiration in §27: a maintained, reference-quality, extensible agent harness that labs, builders, researchers, and products can rely on. The core invariant from §26:

> Everything important that happens in an agent run should be represented as a typed event, attached to a session, governed by capabilities, and reproducible through a stable runtime model.

---

## Alpha status

Roder is alpha software. The core types in `roder-api`, the protocol in `roder-protocol`, and the extension surfaces are stabilizing but not frozen. Expect breaking changes during the rewrite from the legacy Go implementation. The roadmap is the source of truth for what is built, what is in flight, and what is queued.

---

## Repository layout

The Rust workspace under `crates/` follows the boundaries laid out in whitepaper §21:

```
crates/
  roder-api/              Stable native extension traits and canonical types.
  roder-core/             Agent loop, lifecycle, cancellation, event ordering, permission enforcement.
  roder-protocol/         App-server protocol, schemas, generated client surfaces.
  roder-app-server/       Embedded local control plane.
  roder-extension-host/   Native extension installation and provider selection.
  roder-inference/        InferenceEngine + WireDialect surface and model registry.
  roder-context/          ContextProvider, ContextPlanner, context budgets, context blocks.
  roder-session/          SessionStore, CheckpointStore, transcript model, replay primitives.
  roder-memory/           Memory store surface.
  roder-tools/            ToolSpec, ToolExecutor, tool routing, built-in tools.
  roder-sandbox/          Filesystem/process/network/secret brokers.
  roder-cli/              Command entrypoint and distribution wiring.
  roder-tui/              Reference terminal UI client.

  roder-ext-openai-responses/         Responses-style inference engine.
  roder-ext-openai-chat-completions/  Chat-Completions-style inference engine.
  roder-ext-anthropic/                Anthropic Messages-style inference engine.
  roder-ext-gemini/                   Native Gemini inference engine.
  roder-ext-jsonl-session/            JSONL session/checkpoint storage.
  roder-ext-disk-context/             On-disk context persistence.
  roder-ext-memory/                   Local memory extension.
```

The dependency direction is strict, per whitepaper §7:

```
extensions  -> roder-api
core        -> roder-api
apps        -> roder-protocol
roder-cli   -> core + selected extensions
```

Extensions never depend on `roder-core`; the core never depends on any extension. Distribution binaries compose the core with whichever extensions they choose to install.

---

## The roadmap

The roadmap lives in `roadmap/`. It is organized as phased implementation plans, each scoped tight enough to be assigned to a single agent. The index in [`roadmap/00-feature-inventory-and-sequencing.md`](./roadmap/00-feature-inventory-and-sequencing.md) lists every plan and its dependency gates.

Foundational and architectural plans:

- [`roder_rust_rewrite_plan.md`](./roadmap/roder_rust_rewrite_plan.md) — the overall Rust rewrite plan.
- [`roder_extensibility_goals_and_foundations.md`](./roadmap/roder_extensibility_goals_and_foundations.md) — the doctrine extension authors must follow.

Phased implementation plans (selected highlights — see the index for the full list):

- 01: Config, provider, and model catalog.
- 02: Session and message store.
- 03: Tool permissions and hooks.
- 04: Context, skills, and commands.
- 05: MCP and LSP capabilities.
- 06: TUI chat product.
- 07: App-server, headless, and remote.
- 11: Disk session and resume.
- 12: OpenAI Responses with compaction.
- 13: Anthropic Messages support.
- 17: Chat Completions custom models.
- 21: Native Gemini through Google GenAI SDK.
- 22: Web search extensions (Firecrawl, Perplexity, Tavily, Parallel).
- 23: Subagent dispatch (`task` tool, disk-defined agents).
- 24: Plan mode and permission policy modes.
- 25: Workspace checkpoint and undo.
- 26: Custom slash commands.
- 27: TUI status line, command palette, and diff viewer.
- 28: Background tasks and notifications.
- 29: TUI mouse interactions, hover, and clickable elements.

Each plan carries a "Whitepaper Alignment" section mapping its work to the relevant whitepaper chapters and listing the invariants it preserves.

---

## Quick start

The Rust binary is the primary entry point. During the rewrite the legacy Go implementation (see [Relationship to gode](#relationship-to-gode) below) still ships in parallel.

```sh
cargo build --workspace
cargo run -p roder-cli -- --help
cargo test --workspace
```

Web search provider setup is documented in [`docs/roder-web-search-extensions.md`](./docs/roder-web-search-extensions.md).

Subagent setup for the `task` tool and disk-defined agents is documented in [`docs/roder-subagents.md`](./docs/roder-subagents.md).

The terminal UI keymap and `?` help overlay are documented in [`docs/roder-tui-keymap.md`](./docs/roder-tui-keymap.md).

A more complete quick-start (configuration, providers, session resume, app-server transports, MCP) will land alongside the corresponding roadmap phases.

### Legacy Go remote app-server

The Go app-server can expose the same local protocol over a token-authenticated WebSocket for same-network or Tailscale clients:

```sh
go run ./cmd/gode app-server --remote --provider mock --model mock
go run ./cmd/gode app-server --remote --listen ws://100.x.y.z:0 --print-qr=false
GODE_REMOTE_TOKEN=... go run ./cmd/gode app-server --remote --auth-token env:GODE_REMOTE_TOKEN --remote-token-ttl 1h
```

Remote mode defaults to `ws://0.0.0.0:0`, prints usable connect URLs, and emits a `gode://connect?payload=...` QR payload. The WebSocket URL does not carry the bearer token in its query string; native clients should send `Authorization: Bearer <token>`, while browser-constrained clients can use subprotocols `gode.remote.v1, bearer.<token>`.

Only `/readyz` and `/healthz` are unauthenticated. WebSocket upgrades require the token, Origin headers are rejected unless passed with `--allowed-origin`, and remote events/logs redact token material. Raw LAN WebSockets do not provide TLS; prefer a Tailscale address on shared networks or treat the bearer token as the full access secret.

---

## Extension authoring sketch

The basic shape of a native extension, per whitepaper §8:

```rust
use roder_api::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};

pub struct MyInferenceExtension;

impl RoderExtension for MyInferenceExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "com.example.my-inference".into(),
            name: "MyInference".into(),
            version: semver::Version::new(0, 1, 0),
            api_version: "v1".into(),
            description: Some("Example inference engine extension".into()),
            provides: vec![ProvidedService::InferenceEngine("my-inference".into())],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(std::sync::Arc::new(MyInferenceEngine::new()));
        Ok(())
    }
}
```

A distribution binary composes the core with whatever extensions it wants:

```rust
fn main() -> anyhow::Result<()> {
    roder_cli::run(|registry| {
        registry.install(roder_ext_openai_responses::extension())?;
        registry.install(roder_ext_anthropic::extension())?;
        registry.install(roder_ext_jsonl_session::extension())?;
        registry.install(MyInferenceExtension)?;
        Ok(())
    })
}
```

No fork is required. The lab or product builds its own distribution.

---

## Relationship to `gode`

This repository started as `gode` — a Go-native TUI coding agent and event-driven harness in `internal/godex` and `internal/tui`. That implementation proved the core behaviors Roder is built around: event-driven sessions, the provider abstraction, the tool registry, the MCP manager, the JSONL journal, and the app-server protocol with stdio and WebSocket transports.

Roder is the Rust rewrite of that work, re-founded as a harness-first project rather than a coding-agent application. The Go implementation continues to live under `cmd/gode` and `internal/` during the transition and serves as the behavioral oracle while Rust crates fill out. New work should land in `crates/roder-*`; the roadmap calls out plans whose Go counterparts are explicitly being ported.

See [`roadmap/roder_rust_rewrite_plan.md`](./roadmap/roder_rust_rewrite_plan.md) for the rewrite sequence and which Go behaviors are mapped to which Rust crates.

---

## Status, governance, and contributing

Roder is open source and forkable, but designed so that forking is rarely necessary (whitepaper §20). The project favors:

- stable extension APIs over churn in `roder-api`;
- canonical internal types over provider-specific shapes in `roder-core`;
- provider crates outside the core;
- typed events on the canonical bus over ad hoc logging;
- capability-scoped brokers over raw filesystem/process access;
- app-server methods over UI-only behavior.

The roadmap files are the working contract for in-flight changes. New ideas that affect canonical types or the extension surface should land as a roadmap plan first so dependency gates and invariants are explicit before code moves.

---

## License

To be announced. Roder is intended to ship under a permissive open-source license once a `LICENSE` file lands at the repo root.
