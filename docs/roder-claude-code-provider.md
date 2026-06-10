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

## Session reuse and automatic compaction

The provider keeps overflow under control with two layers:

**1. CLI session reuse (default on).** Instead of replaying the whole transcript every turn, the provider resumes the persisted `claude` CLI session (`ClaudeAgentOptions::resume`) and sends only the new transcript tail — the items after the previous turn's final assistant message. The CLI then keeps history server-side and applies its *own* auto-compaction, so a long thread no longer rebuilds a multi-hundred-thousand-token prompt on each request. Mechanics:

- The provider records the session id returned on `turn/completed` plus a fingerprint of the transcript prefix the session is known to contain (`SessionContinuity` in `crates/roder-ext-claude-code/src/provider.rs`).
- A turn resumes only when there is a stored session id and the current transcript still extends that fingerprinted prefix. If Roder rewrote the head (e.g. its own compaction inserted a `ContextCompaction` summary) or a new conversation started, the prefix check fails and the provider falls back to a fresh full-transcript send, starting a new session.
- Any turn error clears the stored session so the next attempt replays the full transcript rather than resuming a stale/invalid session.
- Set `ClaudeCodeConfig::reuse_cli_session = Some(false)` to force the legacy behavior of replaying the full transcript every turn.

**2. Client-side compaction (safety net).** Because session reuse can fall back to a full send, the fresh-send path still needs to fit. claude-code models are catalogued with `supports_compaction = false`, so Roder proactively summarizes the transcript once the estimated token count reaches the model's `auto_compact_token_limit` (e.g. 900k for the 1M-window aliases) instead of waiting for the full context window. If a turn still fails with a context-overflow error (`Prompt is too long`, `Prompt too long`, `input exceeds`, or `context window`), the next turn force-compacts before resending.

When `file_backed_dynamic_context` is enabled, the pre-compaction transcript is written to a chat-history artifact so earlier details remain recoverable via `read_artifact`.

> Note: this client-side compaction is specific to the `claude-code` CLI provider. The direct **`anthropic`** API-key provider instead uses Anthropic's native server-side compaction: those models are catalogued with `supports_compaction = true`, and Roder forwards `auto_compact_token_limit` as a `context_management` edit (`compact_20260112`, beta header `anthropic-beta: compact-2026-01-12`) so the server summarizes older turns once input crosses the trigger (clamped to Anthropic's 50k-token minimum). See `crates/roder-ext-anthropic/src/provider.rs`.

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
