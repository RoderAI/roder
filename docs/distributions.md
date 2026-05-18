# Roder Distributions

Roder treats distributions as first-class artifacts: a distribution is a small binary crate that composes `roder-cli`, optional UI/control-plane crates, and the selected extension crates. This is the concrete path for the whitepaper section 8 idea that labs and products build distributions, and the section 20 goal that forking core should be rare.

The configurator binary is `roder-configure`.

## Built-In Profiles

- `minimal`: CLI/TUI with JSONL sessions, plan mode, and terminal notifications.
- `openai-only`: OpenAI Responses, JSONL sessions, plan mode, terminal notifications, CLI, TUI, and app server.
- `anthropic-only`: Anthropic Messages, JSONL sessions, plan mode, terminal notifications, CLI, TUI, and app server.
- `research-headless`: no TUI; app server, OpenAI Responses, disk context, memory, subagents, process tasks, plan mode, and terminal notifications.
- `full`: all first-party extension metadata currently declared in the workspace.

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
  "jsonl-session",
  "plan-mode",
  "notify-terminal",
]
default_provider = "openai-responses"
default_session_store = "jsonl-session"
```

Profiles are strict: unknown fields are rejected. Validation checks that selected extension ids exist, default provider/session-store ids are selected, single-select storage categories are not duplicated, and declared conflicts are caught before generation.

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

Use the `openai-only` profile when the lab wants a reproducible binary that only ships the OpenAI Responses inference path plus local JSONL sessions.

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

The profile includes JSONL replay-oriented sessions, disk context, memory, subagents, and process tasks.

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
  "jsonl-session",
  "plan-mode",
  "notify-terminal",
]
default_provider = "openai-responses"
default_session_store = "jsonl-session"
```

Then run:

```sh
cargo run -p roder-configure -- validate customer-profile.toml
cargo run -p roder-configure -- generate --profile customer-profile.toml --out dist/customer-roder --build
```

The generated crate is the customization boundary; no Roder core fork is needed.
