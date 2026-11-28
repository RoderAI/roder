## 0.3.1 (2026-11-28)

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

## 0.2.1 (2026-10-25)

### Fixes

#### Publish SQLite dependency alignment for registry builds

Release the automations, code index, and memory packages with the workspace's
rusqlite 0.41 dependency. Their previous published versions still require
rusqlite 0.40, which conflicts with the native SQLite linkage used by the
new usage-analytics package.

This makes the existing dependency refresh available to crates.io consumers.
No runtime behavior or previously published tags are changed.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
