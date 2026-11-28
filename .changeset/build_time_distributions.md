---
roder: minor
roder-extension-host: minor
roder-ext-web-search: minor
roder-tools: minor
roder-app-server: minor
roder-evals: minor
roder-configure: minor
roder-dist-hosted: patch
roder-api: minor
roder-ext-codex-backend: patch
roder-ext-zerolang: patch
roder-ext-antigravity-backend: patch
roder-web-search: minor
roder-config: patch
roder-core: patch
roder-ext-memory: patch
roder-ext-zeroentropy-embeddings: patch
---

# Choose Roder's extensions at build time

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
