# Roder Distributions

Roder treats distributions as first-class artifacts: a distribution is a small binary crate that composes `roder-cli`, optional UI/control-plane crates, and the selected extension crates. This is the concrete path for the whitepaper section 8 idea that labs and products build distributions, and the section 20 goal that forking core should be rare.

The configurator binary is `roder-configure`.

## Build-Time Features

What a Roder binary contains is decided at build time by Cargo features on `roder` (the CLI crate) and `roder-extension-host`. A feature that is off leaves its extension out of the build, so its tools, providers, and commands never appear.

- `base` (the default) is what the released `roder` binary, npm package, and Homebrew formula ship: Anthropic, Claude Code, Gemini, xAI, OpenRouter, and roder.cloud, the Docker runner, and the cua desktop tools (still off until `[cua] enabled = true`). Every build also includes OpenAI Responses and Codex, custom OpenAI-compatible providers, git, plan mode, the task ledger, verification, subagents, process tasks, process-hosted extensions, the local runner, MCP, the inference router, terminal and desktop notifications, GitHub review, fork rift, JSONL sessions, and web search through Tavily.
- `full` turns on every extension.
- Each opt-in extension has its own feature:
  - Providers: `vertex`, `opencode`, `fireworks`, `poolside`, `cursor`, `xiaomi-mimo`, `kimi-code`, `synthetic`, `deepseek`, `hopper`, and (CLI only) `antigravity`.
  - Tools: `zerolang`, `design`, `webwright`, `jev`, `chrome`, `browser-use`, `knowledge`, `memory`, `honcho`, `embeddings`, `images`, `speech`.
  - Search backends: `firecrawl-search`, `parallel-search`, `perplexity-search`, `synthetic-search`.
  - Runners: `runner-blaxel`, `runner-cloudflare`, `runner-daytona`, `runner-e2b`, `runner-modal`, `runner-runloop`, `runner-sprites`, `runner-vercel`, or all of them with `hosted-runners`.
  - Session stores: `mysql-session`, `postgres-session`.

Build the default, everything, or a hand-picked set:

```sh
cargo build -p roder --release                                   # base
cargo build -p roder --release --features full                   # everything
cargo build -p roder --release --features zerolang,memory        # base plus two
cargo build -p roder --release --no-default-features --features anthropic
```

Configuration that names a feature this build leaves out fails with a "not built into this Roder distribution" error. This applies to the honcho memory backend, the MySQL and PostgreSQL session stores, an explicit `web_search.provider`, and the `roder zerolang` command. `--browser-use` prints a warning instead.

`embeddings` builds in the OpenAI, Google, and ZeroEntropy embedding providers together; `images` and `speech` likewise each cover their OpenAI and Google providers.

`make test` runs the workspace suite against the full build, so every feature's tests run. `make check-distributions` lints the minimal, base, and full builds and runs the CLI and extension-host tests for minimal and base.

Generated distribution crates depend on `roder` with `default-features = false` and turn on the features for the extensions in their profile.

## Built-In Profiles

- `minimal`: CLI/TUI with JSONL thread storage, plan mode, and terminal notifications.
- `openai-only`: OpenAI Responses, JSONL thread storage, plan mode, terminal notifications, CLI, TUI, and app server.
- `anthropic-only`: Anthropic Messages, JSONL thread storage, plan mode, terminal notifications, CLI, TUI, and app server.
- `research-headless`: no TUI; app server, OpenAI Responses, disk context, memory, subagents, process tasks, plan mode, and terminal notifications.
- `remote-app-server`: no TUI; app server, OpenAI Responses, JSONL sessions, disk context, process tasks, plan mode, and subagents for sandbox-hosted Roder nodes.
- `tavily`: OpenAI Responses plus the web-search router and Tavily-backed search enabled through `TAVILY_API_KEY`.
- `zero-coder-edits`: no TUI; app server, OpenAI Responses, JSONL sessions, disk context, process tasks, plan mode, and the Zerolang checked graph-edit tool provider. Intended for Linux AMD64 RL coding environments that run Zero tasks.
- `full`: all first-party extension metadata currently declared in the workspace, including the Webwright browser-agent extension documented in `docs/roder-webwright-browser-agent.md` and the Zerolang checked graph-edit tool provider documented in `docs/roder-zerolang-checked-graph-edits.md`.

List profiles:

```sh
cargo run -p roder-configure -- profile list
```

Show a bundled profile:

```sh
cargo run -p roder-configure -- profile show openai-only
```

## Headless Usage

Validate a profile:

```sh
cargo run -p roder-configure -- validate ./profile.toml
```

Generate a distribution crate:

```sh
cargo run -p roder-configure -- generate --profile ./profile.toml --out ./dist/lab-roder
```

Emit structured output for CI:

```sh
cargo run -p roder-configure -- --format json validate ./profile.toml
```

Generate and attempt a release build:

```sh
cargo run -p roder-configure -- generate --profile ./profile.toml --out ./dist/lab-roder --build
```

Set `RODER_CONFIGURE_OFFLINE=1` to pass `--offline` to Cargo during the build step.

## Custom Profiles

A checked-in profile is normal TOML:

```toml
id = "lab-openai"
description = "OpenAI-only lab distribution"

[distribution]
name = "lab-openai-roder"
version = "0.1.0"
include_tui = true
include_app_server = true
include_cli = true
extensions = [
  "openai-responses",
  "jsonl-thread-store",
  "plan-mode",
  "notify-terminal",
]
default_provider = "openai-responses"
default_thread_store = "jsonl-thread-store"
```

Profiles are strict: unknown fields are rejected. Validation checks that selected extension ids exist, default provider/thread-store ids are selected, single-select storage categories are not duplicated, and declared conflicts are caught before generation.

## Capability Declarations

Each first-party extension declares `[package.metadata.roder.distribution]` in its `Cargo.toml`. The metadata includes:

- `id`, `display_name`, `description`, and `category`.
- `default_in_profiles` for built-in profile generation.
- `required_env` and `optional_env`; generated configs never contain secret values.
- `required_capabilities`, such as network or filesystem access.
- `conflicts_with` for profile validation.
- `extension_path` for generated crate wiring.

Catalog inspection:

```sh
cargo run -p roder-configure -- catalog list
cargo run -p roder-configure -- catalog show openai-responses
```

## Worked Examples

### Lab OpenAI Responses Build

Use the `openai-only` profile when the lab wants a reproducible binary that only ships the OpenAI Responses inference path plus local JSONL thread storage.

```sh
cargo run -p roder-configure -- profile show openai-only > profile.toml
cargo run -p roder-configure -- validate profile.toml
cargo run -p roder-configure -- generate --profile profile.toml --out dist/openai-roder
```

Set `OPENAI_API_KEY` at runtime. Do not write the key into `profile.toml` or generated `config.toml`.

### Headless Research Distribution

Use `research-headless` for automation, replay, app-server clients, or RL harnesses that do not need the terminal UI.

```sh
cargo run -p roder-configure -- profile show research-headless > profile.toml
cargo run -p roder-configure -- --format json validate profile.toml
cargo run -p roder-configure -- generate --profile profile.toml --out dist/research-roder
```

The profile includes JSONL replay-oriented thread storage, disk context, memory, subagents, and process tasks.

### Remote App-Server Distribution

Use `remote-app-server` for sandbox-hosted Roder nodes that run unattended behind the remote WebSocket app-server.

```sh
cargo run -p roder-configure -- profile show remote-app-server > remote-app-server-profile.toml
cargo run -p roder-configure -- validate remote-app-server-profile.toml
cargo run -p roder-configure -- generate --profile remote-app-server-profile.toml --out dist/remote-roder --build
```

Publish the Linux build as `remote-roder-<target>` with a matching `.sha256`, for example `remote-roder-x86_64-unknown-linux-gnu`. The `.github/workflows/publish-latest-roder.yml` workflow builds and uploads these Linux remote distro artifacts alongside the default `roder-<target>` binaries on the `latest` GitHub release. Locally, run `RODER_PUBLISH_DRY_RUN=1 make publish` to build the artifacts without uploading. The local publish script sets `RUSTC="$(rustup which rustc)"` for `cargo zigbuild` when rustup is available, matching CI and avoiding Homebrew rustc cross-target failures. Sprites bootstraps download the remote artifact from `https://github.com/RoderAI/roder/releases/download/latest`, install it as `.roder/bin/roder`, and run `roder app-server --remote` as a managed background service.

### Zero Coder Edits RL Build

Use `zero-coder-edits` when the runtime should be headless and focused on Zero source tasks. It ships `zerolang_skills_get`, `zerolang_check`, `zerolang_graph_dump`, `zerolang_graph_view`, `zerolang_fix_plan`, `zerolang_edit`, and `zerolang_graph_roundtrip`.

The generated config allowlists only those Zerolang tools, so generic file-editing tools such as `edit`, `multi_edit`, `write_file`, and `apply_patch` are not advertised to the model.

```sh
cargo run -p roder-configure -- profile show zero-coder-edits > zero-coder-edits-profile.toml
cargo run -p roder-configure -- validate zero-coder-edits-profile.toml
cargo run -p roder-configure -- generate --profile zero-coder-edits-profile.toml --out dist/zero-coder-roder
```

For a publishable Linux AMD64 artifact from macOS with Zig installed:

```sh
./scripts/build-zero-coder-roder-linux-amd64.sh
```

Set `OPENAI_API_KEY` at runtime and either put `zero` on `PATH` or set `RODER_ZERO_BIN=/path/to/zero`.

### Tavily-Enabled Web Search Build

Use `tavily` when you want a normal CLI/TUI/app-server build with external Tavily web search configured. The generated profile enables the web-search router, selects `provider = "tavily"`, and keeps secrets in environment variables.

```sh
cargo run -p roder-configure -- profile show tavily > tavily-profile.toml
cargo run -p roder-configure -- validate tavily-profile.toml
cargo run -p roder-configure -- generate --profile tavily-profile.toml --out dist/tavily-roder --build
```

Set `OPENAI_API_KEY` and `TAVILY_API_KEY` at runtime. `TAVILY_PROJECT` is optional and is read only when set.

### Customer-Facing No-TUI Build

Start from `openai-only`, set `include_tui = false`, keep `include_app_server = true`, and remove extensions the customer should not receive.

```toml
id = "customer-headless"
description = "Customer-facing app-server distribution with no TUI"

[distribution]
name = "customer-roder"
version = "0.1.0"
include_tui = false
include_app_server = true
include_cli = true
extensions = [
  "openai-responses",
  "jsonl-thread-store",
  "plan-mode",
  "notify-terminal",
]
default_provider = "openai-responses"
default_thread_store = "jsonl-thread-store"
```

Then run:

```sh
cargo run -p roder-configure -- validate customer-profile.toml
cargo run -p roder-configure -- generate --profile customer-profile.toml --out dist/customer-roder --build
```

The generated crate is the customization boundary; no Roder core fork is needed.
