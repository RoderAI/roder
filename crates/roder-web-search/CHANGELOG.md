## 0.1.3 (2026-11-28)

### Features

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

## 0.1.2 (2026-06-24)

### Features

#### Manage external web-search providers from the TUI

Extend the "Web search provider" settings submenu to list the external provider
router options (firecrawl, tavily, perplexity, parallel, synthetic) alongside
the hosted modes, showing each provider's enabled and API-key-configured status
and the active selection. Selecting an external provider persists
`[web_search] mode = "external"`, the chosen `provider`, and enables that
provider's sub-section in user config (applies on restart).

The same hosted modes and external providers are also selectable from the
`Ctrl+P` command palette, and both the menu and palette fall back to reading
providers directly from user config when app-server settings are unavailable.

`settings/get` and `settings/set_web_search` now report the external-router
snapshot via `WebSearchSettings { external_enabled, external_provider, providers }`
and accept an optional `external_provider` selection. New `roder-config`
helpers (`save_web_search_external_provider`, `WebSearchConfig::provider_configured`,
`WebSearchConfig::router_snapshot`, `web_search_router_snapshot`) back the
persistence, status checks, and config-only fallback.

Synthetic web search now auto-configures from the synthetic inference provider:
because both share `SYNTHETIC_API_KEY`, pasting the synthetic provider key
(`providers/configure`) makes the synthetic search provider report as
`key configured`, enables its `[web_search.synthetic]` sub-section, and lets it
resolve the borrowed key at runtime — no separate web-search key entry needed.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
