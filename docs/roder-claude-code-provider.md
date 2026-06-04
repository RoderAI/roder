# Roder Claude Code Provider

Roder exposes Claude Code as a first-class inference provider id:

```text
claude-code
```

This provider runs the local Claude Code CLI through the Rust `claude-agent-sdk` crate. It is separate from the direct Anthropic API provider (`anthropic`) and does not use `ANTHROPIC_API_KEY` as Roder provider auth.

## Requirements

- `claude` installed and authenticated locally.
- The local Rust SDK crate at `/Users/pz/w/claude-agent-sdk-rust` while this provider is developed as a path dependency.

Normal tests do not run `claude` and do not require a Claude subscription. Live checks must be explicitly enabled.

## Models

The built-in catalog includes Claude Code harness aliases and full Claude model ids:

```text
claude-code/sonnet
claude-code/opus
claude-code/haiku
claude-code/claude-sonnet-4-6
claude-code/claude-opus-4-8
```

Roder sends the selected model string to the SDK unchanged.

## Configuration

Roder can discover `claude` from `PATH`, or you can configure the CLI path:

```toml
provider = "claude-code"
model = "sonnet"

[providers.claude-code]
cli_path = "claude"
permission_mode = "default"
setting_sources = ["user", "project"]
```

Environment overrides:

```sh
export RODER_PROVIDER=claude-code
export RODER_MODEL=sonnet
export RODER_CLAUDE_CODE_CLI_PATH=claude
export RODER_CLAUDE_CODE_PERMISSION_MODE=default
```

`CLAUDE_CODE_CLI_PATH` and `CLAUDE_CODE_PERMISSION_MODE` are also accepted.

## Supported surface

The initial provider maps SDK streaming events into canonical Roder inference events:

- text chunks -> `MessageDelta`
- thinking chunks -> `ReasoningDelta`
- tool-use starts/deltas -> Roder tool-call events
- tool results and rate-limit notices -> provider metadata
- complete events -> usage and completion metadata when present
- SDK errors -> redacted Roder failure events

Structured `response_format` is rejected before a prompt is sent because the current Claude Code SDK path does not expose a stable structured-output contract.

## Tool safety

Claude Code tool-use events are surfaced through Roder's canonical event stream. The provider installs a default SDK `can_use_tool` callback that denies unmapped Claude Code tool execution, so the harness does not run local filesystem or shell tools outside Roder while same-stream `TurnToolExecutor` mapping is still being hardened. Broader same-stream execution of Claude Code tool requests through `TurnToolExecutor` remains the next hardening area before advertising more aggressive tool autonomy.

## Live checks

Offline checks:

```sh
cargo test -p roder-ext-claude-code
cargo test -p roder-app-server --test e2e providers_list_exposes_claude_code_models_without_api_key -- --nocapture
```

Live checks should remain opt-in and redacted:

```sh
RODER_CLAUDE_CODE_LIVE=1 \
  cargo test -p roder-ext-claude-code live_claude_code -- --ignored --nocapture
```
