# Roder Distributions

Roder treats distributions as first-class artifacts: a distribution is a small binary crate that composes `roder-cli`, optional UI/control-plane crates, and the selected extension crates. This is the concrete path for the whitepaper section 8 idea that labs and products build distributions, and the section 20 goal that forking core should be rare.

The configurator binary is `roder-configure`.

## Built-In Profiles

- `minimal`: CLI/TUI with JSONL thread storage, plan mode, and terminal notifications.
- `openai-only`: OpenAI Responses, JSONL thread storage, plan mode, terminal notifications, CLI, TUI, and app server.
- `anthropic-only`: Anthropic Messages, JSONL thread storage, plan mode, terminal notifications, CLI, TUI, and app server.
- `research-headless`: no TUI; app server, OpenAI Responses, disk context, memory, subagents, process tasks, plan mode, and terminal notifications.
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

### Zero Coder Edits RL Build

Use `zero-coder-edits` when the runtime should be headless and focused on Zero source tasks. It ships `zerolang_skills_get`, `zerolang_check`, `zerolang_graph_dump`, `zerolang_graph_view`, `zerolang_fix_plan`, `zerolang_edit`, and `zerolang_graph_roundtrip`.

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
