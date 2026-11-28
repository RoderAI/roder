## 0.3.3 (2026-11-28)

### Fixes

#### Choose Roder's extensions at build time

Each opt-in extension is now a Cargo feature on `roder` and
`roder-extension-host`. A build that leaves a feature off leaves that
extension out, along with its tools, providers and commands.

The default `base` build is what the released binary ships:

- Providers: OpenAI, Codex, Anthropic, Claude Code, Gemini, xAI, OpenRouter
  and roder.cloud.
- Core tools and extensions: plan mode, tasks, subagents, MCP,
  notifications, GitHub review, web search through Tavily, and the local and
  Docker runners.

The following are opt-in and no longer in the released binary:

- Providers: Vertex, OpenCode, Fireworks, Poolside, Cursor, Xiaomi MiMo,
  Kimi Code, Synthetic, DeepSeek and Hopper, plus the Antigravity backend.
- Tools: Zerolang, the `design_*` tools, Webwright, Jev, Chrome, browser-use
  and the desktop (cua) tools.
- Memory (including Honcho) and knowledge.
- Embeddings, image generation and speech.
- The Firecrawl, Parallel, Perplexity and Synthetic search backends.
- Hosted runners. `roder-dist-hosted` builds `base` plus the hosted runners.
- The MySQL and PostgreSQL session stores.

`--features full` builds everything. `make check-distributions` lints the
minimal, base and full builds. `roder-configure` now generates crates that
depend on `roder` with `default-features = false` and only the features
their profile needs.

The honcho memory backend, the MySQL and PostgreSQL session stores, an
explicit `web_search.provider` and `roder zerolang` fail with a "not built
into this Roder distribution" error when the build leaves them out.
`--browser-use` prints a warning instead.

API changes:

- `DefaultRegistryConfig::zerolang` takes `roder_config::ZerolangConfig`.
- The `SessionStoreConfig` Postgres and MySQL variants exist only with their
  features.
- The `design_*` tools moved from the built-in coding tools to
  `roder_tools::DesignToolsContributor`.

Starting the Codex app-server, the Antigravity CLI or the Zero binary now retries briefly when
the executable is still open for writing ("Text file busy"), as happens
right after it is updated in place. The retry is
`roder_api::spawn::spawn_retrying_busy`.

Other fixes:

- A config file Roder cannot read or parse now names its path in the error.
- `RedactedHttpError::body` is boxed.

## 0.3.2 (2026-11-27)

### Fixes

- Fix every `cargo clippy --workspace --all-targets -D warnings` error. The fixes are behavior-preserving lint cleanups (collapsed `if let` chains, removed no-op conversions, derived `Default`s, test modules moved after the items they test). Targeted `#[allow]`s with reasons cover wire types and const table constructors.

## 0.3.1 (2026-11-27)

### Fixes

#### Configure GPT and Codex context windows from TUI Settings

Use 500k-token default working context windows for GPT models that support
them. Add a persisted `model_context_window` preference with a numeric TUI
editor and reset-to-default action. Clamp each model's effective window to
its supported maximum and apply the same budget to context counters and
runtime compaction.

Rust integrators must rebuild against the updated Config, RuntimeConfig,
and settings response structs, which add the optional context-window
preference. Release their compatible reverse dependencies together.

## 0.3.0 (2026-10-27)

### Breaking Changes

#### Release a coherent runtime API compatibility boundary

Adopt explicit shared-read/exclusive/coordinator scheduling, typed custom-provider
capabilities, and owned worktree metadata. Rust integrators must update
ToolExecutor implementations to execution_mode and initialize the new public
configuration and ownership fields.

Release the shared API's reverse dependency closure together so older consumers
cannot resolve patch-compatible packages that expose incompatible API types.
The CLI package also exports a Rust library, so its compatibility release is
0.4.0. Existing command-line use remains supported.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.2 (2026-09-26)

### Fixes

#### Publish today's content under fresh crate versions

`roder-api` 0.1.21, `roder-core` 0.1.19, `roder-app-server` 0.1.17,
`roder-evals` 0.1.2 and `roder-ext-openai-responses` 0.1.10 were published to
crates.io on 2026-09-23 from a branch that predates the Codex agent backend,
the Jev browser tool, the Chrome bridge work and the Grok 4.7 catalog entries.
crates.io versions are immutable, so those numbers cannot carry the current
code and the registry copies do not match the git tags of the same version.

Bump them so the released content reaches crates.io under versions that
describe it. `roder-ext-jev` and `roder-ext-codex-backend` are bumped with
them: both are new crates whose first publish must depend on a registry
`roder-api` that actually contains `roder_api::backend`, which the 0.1.21
registry copy does not.

Adds the package-local READMEs both new crates need to publish.

## 0.1.1 (2026-09-26)

### Features

#### Select Codex as a complete agent backend for Roder

Add an extension service for complete agent backends and a Codex app-server implementation. Roder app-server maps its thread, turn, streaming event, and approval APIs to Codex, so existing Roder clients and the TUI can use Codex as the agent runtime.

Refresh GPT-6 and Claude Opus 5.5/Sonnet 5 catalog entries and map Codex token usage into Roder's turn counters.
