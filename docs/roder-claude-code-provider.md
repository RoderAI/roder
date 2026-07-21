# Roder Claude Code Provider

Roder exposes Claude Code as a first-class inference provider id:

```text
claude-code
```

This provider runs the local Claude Code CLI through the Rust `claude-agent-sdk` crate. It is separate from the direct Anthropic API provider (`anthropic`) and does not use `ANTHROPIC_API_KEY` as Roder provider auth.

## CLI process lifecycle

Roder vendors a narrow, MIT-licensed patch of `claude-code-sdk-rust` 0.4.0 under
`vendor/claude-code-sdk-rust` and applies it through Cargo's
`[patch.crates-io]` mechanism. The patch configures the spawned `claude` child
with `kill_on_drop(true)`, makes the convenience stream observe receiver drop,
and always disconnects the owned SDK client before the stream task exits. The
transport close path kills and waits for the child process. This gives an
interrupted Roder turn an SDK-level child-process cleanup path rather than
relying only on Tokio task abort.

The vendor patch provenance is recorded in
`vendor/claude-code-sdk-rust/RODER_PATCH.md`. Roder intends to remove the
override after an upstream released SDK contains equivalent receiver-close-aware
stream cleanup and subprocess ownership behavior.

Roder still treats cleanup as bounded and observable, not guaranteed solely by
this provider. The local TUI first requests `turn/interrupt`, restores the
terminal, then the CLI performs a bounded `runtime/drain`. The CLI retains a
final `std::process::exit(0)` guard after that drain while optional third-party
subprocess providers cannot all prove equivalent cleanup. A non-`clean` drain
warning means some locally owned turn or process work was still present, or
lifecycle persistence failed, when the deadline expired.

## Requirements

- `claude` installed and authenticated locally.
- The vendored `claude-code-sdk-rust` patch included with this Roder source
  tree. Cargo applies it automatically; a separate local SDK checkout is not a
  runtime requirement.

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

## Claude in Chrome (browser tools)

Claude Code ships a native "Claude in Chrome" (CFC) integration: when the
`claude` CLI runs under your claude.ai subscription with the Chrome extension
paired, the model gets browser tools named `mcp__claude-in-chrome__*`
(`navigate`, `read_page`, `javascript_tool`, `tabs_context_mcp`, ...) that drive
your real local browser.

Roder's provider can register against that integration so a `claude-code/*` turn
can use the browser:

- It spawns `claude` with `CLAUDE_CODE_ENABLE_CFC=1` so the CLI wires its
  Claude-in-Chrome MCP server even in the SDK's headless/streaming mode (the CLI
  only auto-wires it for interactive sessions otherwise).
- It blanks `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` for that child process.
  The Chrome integration only connects under claude.ai subscription auth; an
  inherited API key takes precedence and disables it. Blanking the key for the
  child makes the CLI fall back to its subscription login.
- It pre-authorizes the `mcp__claude-in-chrome__*` tools through the SDK
  `can_use_tool` callback (every other unmapped CLI tool stays denied).
- Because the CLI executes these tools itself, the provider surfaces them as
  hosted tool calls (`HostedToolCallStarted` / `HostedToolCallCompleted`) so the
  activity shows in the UI without the runtime trying to re-run a tool it never
  registered.

### Enabling

Enablement is resolved per turn in this order:

1. `ClaudeCodeConfig::enable_claude_in_chrome` (`Some(true)` / `Some(false)`).
2. The `RODER_CLAUDE_CODE_ENABLE_CHROME` or `CLAUDE_CODE_ENABLE_CHROME`
   environment variable (`1`/`true`/`on` or `0`/`false`/`off`).
3. Auto-detection (default): on when the local Claude Code config
   (`$CLAUDE_CONFIG_DIR/.claude.json` or `~/.claude.json`) shows the Chrome
   extension is paired or enabled (`claudeInChromeDefaultEnabled`,
   `chromeExtension.pairedDeviceId`, or `cachedChromeExtensionInstalled`).

```sh
# Force the integration on (or off) regardless of detection:
export RODER_CLAUDE_CODE_ENABLE_CHROME=1
```

To actually act on a page, Chrome must be running with the paired Claude
extension; otherwise the tool still runs but reports the extension as not
connected.

> Note: enabling this means a `claude-code` turn uses your claude.ai
> subscription auth, not `ANTHROPIC_API_KEY`. Set
> `RODER_CLAUDE_CODE_ENABLE_CHROME=0` if you need the CLI to keep using an API
> key.

## Live checks

Offline checks:

```sh
cargo test -p roder-ext-claude-code
cargo test -p roder-app-server --test e2e providers_list_exposes_claude_code_models_without_api_key -- --nocapture
```

The vendored SDK also has an offline fake-CLI regression that drops a stream
receiver and proves its owned child is terminated and reaped. Run it from a
temporary standalone copy of `vendor/claude-code-sdk-rust` because that vendor
manifest is intentionally outside the root workspace.

Live checks should remain opt-in and redacted:

```sh
RODER_CLAUDE_CODE_LIVE=1 \
  cargo test -p roder-ext-claude-code live_claude_code -- --ignored --nocapture
```

The Claude-in-Chrome path has its own opt-in live check (needs an authenticated
`claude` CLI with the Chrome extension paired):

```sh
RODER_CLAUDE_CODE_CHROME_LIVE=1 RODER_CLAUDE_CODE_MODEL=sonnet \
  cargo test -p roder-ext-claude-code --test live_claude_in_chrome -- --ignored --nocapture
```
