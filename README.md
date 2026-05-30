# Roder

**Rust Coder.** A Rust-native, extension-first agent harness for coding agents, research systems, reinforcement-learning environments, and AI-native developer tools.

> Roder is the core harness. Everything else is a distribution, extension, provider, policy, interface, or experiment.

Roder is not just another coding agent. It is the substrate underneath them — a stable, strongly typed Rust runtime that handles inference orchestration, tool execution, capability-scoped filesystem and process access, context assembly, thread persistence, checkpointing, policy enforcement, event streaming, and replay, with extension points designed so that new model APIs, storage systems, context engines, sandbox backends, UIs, and training environments can be added without forking the core.

The full mission, design philosophy, and architectural commitments live in [`WHITEPAPER.md`](./WHITEPAPER.md). The phased implementation plan lives in [`roadmap/`](./roadmap/), entry point [`roadmap/00-feature-inventory-and-sequencing.md`](./roadmap/00-feature-inventory-and-sequencing.md).

---

## What Roder represents

Most agent systems today are rebuilt repeatedly around the same primitives — model invocation, tool execution, sandboxing, context, thread persistence, policy, event streaming, replay — usually in Python or Node.js. That works for fast iteration but fragments the ecosystem: every team re-solves the same harness problems in incompatible ways, and labs end up forking upstream because the architecture cannot express the modifications they need.

Roder's bet is that this is unnecessary. The AI ecosystem should not need to rewrite the same agent harness every time a new model, product, research direction, or interaction paradigm appears.

Roder aims to be that foundation, once, with:

- **A stable core that owns invariants.** Lifecycle ordering, cancellation, event ordering, permission enforcement, tool routing, and capability checks live in `roder-core` and are not extensible — extensions provide behavior, but they cannot corrupt the runtime.
- **A native extension kernel.** Inference engines, wire dialects, context providers, context planners, thread stores, checkpoint stores, memory backends, policy contributors, sandbox backends, event sinks, and tool contributors all install through a single `RoderExtension` trait against `roder-api`.
- **Canonical internal representations.** Threads, turns, transcript items, tool calls, inference events, file changes, context blocks, checkpoints, and policy decisions are typed Roder concepts. Provider extensions translate to and from these canonical types; the core never sees Responses, Chat Completions, or Anthropic Messages wire formats.
- **Capability-based access.** Tools and extensions receive `ScopedFilesystem`, `ScopedProcessRunner`, `ScopedNetwork`, `ScopedSecrets`, `ApprovalClient`, and `EventSink` handles. There is no ambient `std::fs` or `std::process::Command` access.
- **Event-sourced execution.** Every meaningful runtime transition is a typed event on the canonical bus. This makes Roder replayable, auditable, resumable, and usable as an RL trajectory substrate without bespoke instrumentation.
- **App-server-first control plane.** The TUI is a client of the embedded local app server. IDE plugins, web UIs, headless CI runners, RL harnesses, and IDE extensions all speak the same control-plane protocol.

The whitepaper lays out the long-term aspiration in §27: a maintained, reference-quality, extensible agent harness that labs, builders, researchers, and products can rely on. The core invariant from §26:

> Everything important that happens in an agent run should be represented as a typed event, attached to a thread, governed by capabilities, and reproducible through a stable runtime model.

---

## Alpha status

Roder is alpha software. The core types in `roder-api`, the protocol in `roder-protocol`, and the extension surfaces are stabilizing but not frozen. Expect breaking changes while the Rust runtime and extension APIs settle. The roadmap is the source of truth for what is built, what is in flight, and what is queued.

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
  roder-thread-store/     ThreadStore, checkpoint storage, transcript model, replay primitives.
  roder-memory/           Memory store surface.
  roder-tools/            ToolSpec, ToolExecutor, tool routing, built-in tools.
  roder-sandbox/          Filesystem/process/network/secret brokers.
  roder-cli/              Command entrypoint and distribution wiring.
  roder-tui/              Reference terminal UI client.

  roder-ext-openai-responses/         Responses-style inference engine.
  roder-ext-openai-chat-completions/  Chat-Completions-style inference engine.
  roder-ext-anthropic/                Anthropic Messages-style inference engine.
  roder-ext-gemini/                   Native Gemini inference engine.
  roder-ext-jsonl-thread-store/       JSONL thread/checkpoint storage.
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

`roder roadmap` is the local control surface for that work. Use `roder roadmap board [plan]` to see plan progress, task readiness, validation state, and attached worker lanes; `roder roadmap dispatch <plan> [task-id]` to print the focused worker brief; and `roder roadmap spawn <plan> [task-id]` to allocate a tracked worker attachment before steering or attaching a live thread. Opening `roder roadmap` or `/roadmap` in the TUI shows the same plan-first operator surface.

Foundational and architectural plans:

- [`roder_rust_rewrite_plan.md`](./roadmap/roder_rust_rewrite_plan.md) — the overall Rust rewrite plan.
- [`roder_extensibility_goals_and_foundations.md`](./roadmap/roder_extensibility_goals_and_foundations.md) — the doctrine extension authors must follow.

Phased implementation plans (selected highlights — see the index for the full list):

- 01: Config, provider, and model catalog.
- 02: Thread and transcript store.
- 03: Tool permissions and hooks.
- 04: Context, skills, and commands.
- 05: MCP and LSP capabilities.
- 06: TUI chat product.
- 07: App-server, headless, and remote.
- 11: Disk threads and resume.
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

The Rust binary is the primary entry point.

```sh
cargo build --workspace
cargo run -p roder-cli --bin roder -- auth status
cargo test --workspace
```

This repo also includes a mise configuration for repeatable local tooling:

```sh
mise install
mise run rust:test
mise run python:test
mise run ci
```

Useful focused tasks include `mise run rust:fmt`, `mise run rust:clippy`,
`mise run python:sync`, and `mise run python:test:startup`.

Release policy lives in [`docs/releases.md`](./docs/releases.md). Release-plz
opens the version-bump PR for Cargo workspace changes and creates git-only
releases after that PR merges.

Install the standard Roder CLI locally with:

```sh
curl -fsSL https://dl.roder.sh/latest/install.sh | sh
```

For source builds:

```sh
make install
```

By default both installers write `roder` to `~/.local/bin`. Override the binary installer with `RODER_INSTALL_DIR=/path/to/bin`; override the source install target with `BINDIR=/path/to/bin make install`.

Roder reads configuration and user state from `~/.roder` by default. Use the
global `--config-dir <path>` flag before or after the subcommand to run against
an alternate directory:

```sh
roder --config-dir /tmp/lab-roder auth status
roder app-server --config-dir /tmp/lab-roder --listen stdio://
```

## Roder distribution configurator

`roder-configure` generates a small downstream distribution crate, an initial `config.toml`, and optionally builds the resulting binary. It is the no-fork path for labs and products that want a tailored Roder build.

Built-in profiles:

- `minimal`
- `openai-only`
- `anthropic-only`
- `research-headless`
- `tavily`
- `full`

```sh
cargo run -p roder-configure -- profile list
cargo run -p roder-configure -- profile show openai-only
cargo run -p roder-configure -- profile show tavily > tavily-profile.toml
cargo run -p roder-configure -- validate ./profile.toml
cargo run -p roder-configure -- generate --profile ./profile.toml --out ./dist/lab-roder
```

Structured CI output is available with `--format json`:

```sh
cargo run -p roder-configure -- --format json validate ./profile.toml
```

See [`docs/distributions.md`](./docs/distributions.md) for built-in profiles, custom profile format, capability declarations, and worked examples for OpenAI-only, Tavily-enabled, research-headless, and customer-facing no-TUI distributions.

The `full` profile includes the first-party Webwright browser-agent extension; see [`docs/roder-webwright-browser-agent.md`](./docs/roder-webwright-browser-agent.md) for first-time setup, browser selection, CLI, app-server, artifact, export, and visual-judge details.

The configurator must not write API keys into generated files. Put secrets in environment variables such as `OPENAI_API_KEY`; generated docs and configs list the required env-var names instead of secret values.

## Remote runner destinations

Remote runner configuration is optional. When disabled or omitted, Roder keeps using the local filesystem and local tool execution path.

```toml
[remote_runners]
enabled = false
```

```toml
[remote_runners]
enabled = true
default_destination = "unix-local"

[remote_runners.destinations.unix-local]
provider = "unix-local"
```

Destination-specific provider settings live under `config`, while secrets are referenced by environment variable name under `secret_env` instead of being written directly into `config.toml`:

```toml
[remote_runners]
enabled = true
default_destination = "docker-dev"

[remote_runners.destinations.docker-dev]
provider = "docker"
config = { image = "rust:latest" }
secret_env = { DOCKER_TOKEN = "RODER_DOCKER_TOKEN" }
```

Hosted providers use the same destination shape. Secrets stay in environment variables or secret references; do not put raw tokens in checked-in config:

```toml
[remote_runners]
enabled = true
default_destination = "blaxel-dev"

[remote_runners.destinations.blaxel-dev]
provider = "blaxel"
secret_env = { BLAXEL_API_KEY = "BLAXEL_API_KEY" }
config = { region = "iad" }
```

For local testing, override the selected destination without editing the file:

```sh
RODER_REMOTE_RUNNER=unix-local cargo run -p roder-cli --bin roder
```

The app-server exposes `runners/list`, `runners/select`, `runners/session`, `runners/snapshot`, `runners/delete`, and `runners/ports`. The TUI exposes runner selection from the `Ctrl+P` menu and shows the active runner in the status surface. Runner sessions own files, commands, ports, snapshots, mounts, artifacts, and provider state; Roder orchestrates and persists the selected destination/session boundary.

See [`docs/roder-remote-runners.md`](./docs/roder-remote-runners.md) for mounts, artifacts, snapshots, ports, and secret-handling rules.

OpenAI hosted web search is enabled by default. External web search provider setup is documented in [`docs/roder-web-search-extensions.md`](./docs/roder-web-search-extensions.md).

xAI Grok and SuperGrok provider setup is documented in [`docs/roder-xai-grok-providers.md`](./docs/roder-xai-grok-providers.md). Use `xai/grok-4.3` with `XAI_API_KEY` for direct xAI API-key auth, or select `supergrok/grok-4.3` in the TUI to start SuperGrok OAuth.

OpenRouter provider setup is documented in [`docs/roder-openrouter-provider.md`](./docs/roder-openrouter-provider.md). Use `openrouter/x-ai/grok-build-0.1` with `OPENROUTER_API_KEY`; direct xAI uses `grok-build-0.1`, while OpenRouter uses the `x-ai/grok-build-0.1` slug.

OpenCode provider setup is documented in [`docs/roder-opencode-providers.md`](./docs/roder-opencode-providers.md). Use `opencode/<model>` for OpenCode Zen models or `opencode-go/<model>` for OpenCode Go models, with API keys from [`https://opencode.ai/auth`](https://opencode.ai/auth).

Poolside provider setup is documented in [`docs/roder-poolside-providers.md`](./docs/roder-poolside-providers.md). Use `poolside/laguna-m.1` or `poolside/laguna-xs.2` with `POOLSIDE_API_KEY` or a key stored from the provider menu; API keys are managed at [`https://platform.poolside.ai/api-keys`](https://platform.poolside.ai/api-keys).

Cursor provider setup is documented in [`docs/roder-cursor-provider.md`](./docs/roder-cursor-provider.md). Use `cursor/composer-2.5` with `CURSOR_API_KEY` or `RODER_CURSOR_API_KEY`; Roder exchanges the key and calls Cursor AgentService directly without invoking the Cursor CLI at inference runtime.

Xiaomi MiMo provider setup is documented in [`docs/roder-xiaomi-mimo-providers.md`](./docs/roder-xiaomi-mimo-providers.md). Use `xiaomi-mimo/<model>` with `MIMO_API_KEY` for pay-as-you-go API access, or `xiaomi-mimo-token-plan/<model>` with `MIMO_TOKEN_PLAN_API_KEY` and the exclusive Token Plan base URL. Xiaomi TTS models are exposed through `roder speech synthesis-providers` and `speech/synthesize`, not the text model catalog.

Custom OpenAI-compatible providers can be added with a provider-specific base URL:

```toml
[providers.local-openai]
base_url = "http://localhost:11434/v1"
api_key_env = "LOCAL_OPENAI_API_KEY"
```

Roder discovers models for custom providers in the background by trying `GET <base_url>/models` and then `GET <base_url>/v1/models`, caches successful results in `~/.roder/models-cache.json`, and keeps provider/model picker calls responsive by returning cached models immediately.

App-server docs live under [`docs/app-server/`](./docs/app-server/): [`api.md`](./docs/app-server/api.md) is the integrator-facing JSON-RPC reference, [`protocol.md`](./docs/app-server/protocol.md) summarizes the client contract, and [`remote.md`](./docs/app-server/remote.md) covers remote WebSocket pairing, auth, and security assumptions.

Subagent setup for the `task` tool and disk-defined agents is documented in [`docs/roder-subagents.md`](./docs/roder-subagents.md). Transparent child trace events, app-server trace read/list methods, persistence behavior, and TUI controls are documented in [`docs/roder-subagent-traces.md`](./docs/roder-subagent-traces.md). Plan review artifacts, hunk records, app-server methods, and deferred rollback behavior are documented in [`docs/roder-plan-review-hunk-tracker.md`](./docs/roder-plan-review-hunk-tracker.md). Workflow import for AGENTS.md, skills, MCP, hooks, commands, and plugins is documented in [`docs/roder-workflow-import.md`](./docs/roder-workflow-import.md). Built-in skills, exposure rules, config, and feature bindings are documented in [`docs/roder-built-in-skills.md`](./docs/roder-built-in-skills.md). Plugin marketplace defaults, de-duplicated search, and install commands are documented in [`docs/roder-plugin-marketplaces.md`](./docs/roder-plugin-marketplaces.md). Terminal media generation, artifacts, previews, and generated-image attachments are documented in [`docs/roder-terminal-media-generation.md`](./docs/roder-terminal-media-generation.md). File-backed dynamic context, `read_artifact`/`grep_artifact`/`tail_artifact`, and artifact app-server methods are documented in [`docs/roder-file-backed-dynamic-context.md`](./docs/roder-file-backed-dynamic-context.md). SQLite vector memories, project/global scopes, embedding providers, and memory CLI/app-server controls are documented in [`docs/roder-memories.md`](./docs/roder-memories.md).

Custom model edit-tool preferences can be set in `~/.roder/config.toml`:

```toml
[models."my-openai-compatible-model"]
edit_tool = "patch" # or "edit"
parallel_tool_calls = true # set false for custom models that need serial tool calls
```

`patch` advertises `apply_patch`; `edit` advertises `write_file`, `edit`, and `multi_edit`. Roder never advertises both edit surfaces to a single model request.
Parallel tool calls are enabled by default. For OpenAI Responses-compatible providers, Roder sends the model-specific `parallel_tool_calls` setting with each tool-capable request and executes each returned tool-call batch concurrently unless that model override is set to `false`.

The app-server run-control methods are `turn/start`, `turn/steer`, and `turn/interrupt`. `turn/start` and `turn/steer` accept `input` blocks such as `{ "type": "text", "text": "..." }`. `turn/start` also accepts per-turn `modelProvider`, `model`, `reasoning`, and `policyMode` overrides. Steering accepts `{ "threadId": "...", "expectedTurnId": "...", "input": [...] }`, emits `turn.steered`, and appends the steering message to the active turn before the next provider request.

`settings/get` returns runtime settings including hosted web search, shell command shell, default provider/model/reasoning/policy mode, and file-backed dynamic context. `settings/set_web_search` accepts `{ "mode": "cached" }`, `{ "mode": "live" }`, or `{ "mode": "disabled" }`; `settings/set_shell` accepts `{ "shell": "zsh" }` or another shell binary/path; `settings/set_file_backed_dynamic_context` accepts `{ "enabled": true }` or `{ "enabled": false }`. The TUI exposes these under the Ctrl+P settings menu and the Ctrl+K palette Settings source, and persists choices to `~/.roder/config.toml` when user config persistence is enabled.

The command execution shell defaults to zsh on macOS and bash elsewhere, unless the user's login shell is zsh. To override it in config:

```toml
[tools]
shell = "zsh" # or "bash", "/bin/bash", etc.
```

File-backed dynamic context is enabled by default. To disable it in config:

```toml
[context]
file_backed_dynamic_context = false
```

`tools/list` exposes the built-in coding tools plus Roder workflow helpers: `exec_command`, `write_stdin`, `update_plan`, `get_goal`, `create_goal`, `update_goal`, and `request_user_input`. `exec_command` starts a shell session and returns either final output or a `session_id`; `write_stdin` writes to or polls that session. When a model calls `request_user_input`, Roder emits `thread/userInputRequested` and pauses the turn until a client answers with:

```json
{
  "method": "thread/resolve_user_input",
  "params": {
    "requestId": "user-input-1",
    "answers": { "mode": "Safe" }
  }
}
```

The response is `{ "resolved": true }` when the request was pending. Roder then emits `thread/userInputResolved`, returns the answers to the model as the tool result, and continues the turn.

A more complete quick-start (configuration, providers, thread resume, app-server transports, MCP) will land alongside the corresponding roadmap phases.

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
        registry.install(roder_ext_jsonl_thread_store::extension())?;
        registry.install(MyInferenceExtension)?;
        Ok(())
    })
}
```

No fork is required. The lab or product builds its own distribution.

---

## Relationship to `gode`

This repository started as `gode`, a Go-native TUI coding agent and event-driven harness. That implementation proved the core behaviors Roder is built around: event-driven threads, provider abstraction, tool routing, thread storage, policy modes, and the app-server control plane.

Roder is now the active implementation. The previous Go code has been removed from this repository; new work should land in `crates/roder-*`, `docs/`, `roadmap/`, or the Rust CLI/TUI surfaces.

See [`roadmap/roder_rust_rewrite_plan.md`](./roadmap/roder_rust_rewrite_plan.md) for the historical rewrite sequence and the Rust crate mapping.

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

### PostgreSQL session storage

Roder uses local JSONL thread storage by default. Operators can opt into tenant-scoped PostgreSQL session storage with `RODER_SESSION_STORE=postgres`, `RODER_POSTGRES_SESSION_URL`, and `RODER_POSTGRES_SESSION_TENANT`, or the equivalent `[sessions]` config. See `docs/roder-postgresql-session-store.md` for setup, migration, and troubleshooting details.
