# Changelog

All notable changes to this package will be documented in this file.
## 0.1.2 (2026-11-28)

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

## 0.1.1 (2026-11-27)

### Features

#### Guided Antigravity installation and Google sign-in

Guide first-time Antigravity users through a TUI setup flow before starting a
conversation. Offer a verified, per-user managed runtime download when the CLI
is missing, show install progress, hand off to the official Google sign-in flow,
and verify account access before continuing. Noninteractive entry points never
install software or begin sign-in automatically.

Launch full-access Antigravity sessions with the official
`--dangerously-skip-permissions` option while preserving approval-enabled and
Plan modes.

#### Use Google Antigravity subscriptions through the official ACP runtime

Add `roder --backend antigravity` and subscription sign-in via the official
`agy acp` CLI. Discover models from the account, stream agent activity, and route
tool permissions through Roder without extracting tokens or falling back to
Gemini API-key billing. Make shared backend provider and approval labels work
with runtimes other than Codex.
