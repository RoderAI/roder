## 0.4.5 (2026-11-28)

### Fixes

#### Claude Code sessions get only the Roder tools they need

Claude Code sessions now receive one copy of each core tool, under Claude Code's native name (`Bash`, `Read`, `Edit` and so on). Previously each also came with a second copy under its Roder name (`shell`, `read_file`, `edit`...).

They also no longer receive:

- Roder's browser tools (`chrome_*`, `jev_*`, `browser_use_*`, `webwright.*`). Claude Code has its own browser through Claude-in-Chrome, and the system prompt's browser routing section is dropped with them.
- The `zerolang_*`, `design_*`, `knowledge_*` and `memory_*` tools.
- The Codex-style `exec_command`, `unified_exec` and `write_stdin`, and `apply_patch` when `edit` is also offered. In the `patch` edit mode `apply_patch` is the only editing tool, so it stays.
- The `echo` and `media_generate_video` test tools.

Every tool schema is sent with every request. This cuts Roder's tool list in a Claude Code session from 117 tools to 35, and the schema text from about 76K characters to about 22K.

The browser routing guidance moved from `roder-core` to `roder_api::browser_routing` so providers can remove it.

## 0.4.4 (2026-11-27)

### Fixes

#### Claude Code sessions load only Roder's MCP tools

Claude Code sessions started by Roder now pass `--strict-mcp-config`, so the CLI no longer loads the user's own MCP servers and claude.ai connectors. Roder already denied those tools, but their schemas went into every request. With a few connectors set up, a one-line prompt cost about 160K input tokens; it now costs under 1K. Claude-in-Chrome still works.

## 0.4.3 (2026-11-27)

### Fixes

#### Claude Code context meter shows the context, not the turn's total spend

Claude Code runs its own agent loop, and its turn result sums the usage of
every API request in the turn. The TUI context meter read that sum, so a turn
with one tool call showed about twice the real context (209% of the window on
Claude Code Haiku).

- `TokenUsage` has a new optional `context_tokens` field: the prompt plus
  output of the step's latest model request. `TokenUsage::context_window_tokens()`
  returns it, or `total_tokens` when a provider does not set it.
- The Claude Code provider sets `context_tokens` on the turn's usage from the
  last request it saw. Turn and thread token totals still count every request.
- The TUI context meter and context breakdown use the latest request.
- Claude Haiku 5.5 (`claude-haiku-5-5`, direct and through Claude Code) and the
  Claude Code `haiku` alias now have a 1M token context window, up from 200K.
  Claude Haiku 4.5 stays at 200K.

## 0.4.2 (2026-11-27)

### Features

- Upgrade to `claude-code-sdk-rust` 0.5.1 (parity with the Python Claude Agent SDK v0.2.189). Roder's tools now run as async SDK MCP tools instead of blocking a runtime thread. They opt out of the SDK's new input-schema validation so Claude's native argument shapes are still repaired. A `conversation_reset` from the CLI now ends CLI-session reuse and is reported as provider metadata. An interrupted turn now ends the `claude` child immediately rather than after a 5 s stdin-EOF grace period. The vendored SDK copy is removed: the supervised-stream patch is now part of the upstream release.

## 0.4.1 (2026-11-27)

### Fixes

#### Configure GPT and Codex context windows from TUI Settings

Use 500k-token default working context windows for GPT models that support
them. Add a persisted `model_context_window` preference with a numeric TUI
editor and reset-to-default action. Clamp each model's effective window to
its supported maximum and apply the same budget to context counters and
runtime compaction.

Rust integrators must rebuild against the updated Config, RuntimeConfig,
and settings response structs, which add the optional context-window
preference. Release their compatible reverse dependencies together.

## 0.4.0 (2026-10-27)

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

## 0.3.1 (2026-10-23)

### Fixes

#### Restore reliable Cursor native tool execution

Preserve structured, policy-approved results across provider-native tool loops.
Cursor reads, searches, listings, writes, deletes, and shell replies now use the
runtime executor's output and failures. Add current client-side MCP dispatch,
default runtime tool discovery and MCP state replies, explicit unsupported-call
errors, official turn-end usage, and reliable stream
failure and heartbeat cleanup.

Persist native checkpoints and KV blobs to preserve thread continuity. Serve KV
GETs, prefetch restored blobs, and import Roder history explicitly for new Cursor
conversations and compaction summaries. Use the same bidirectional transport for
text-only API calls and tool-enabled turns.

## 0.3.0 (2026-10-03)

### Breaking Changes

#### Bind hosted external tools to one authenticated connection

Hosted external tool threads require an executor binding before a turn starts.
Only that connection receives execution requests and may resolve them with the
current lease and turn. Disconnect, unbind, and takeover terminate pending
requests. Metadata readback supports recovery without replaying effects.
The TypeScript SDK adds a generic executor helper with cancellation and duplicate
request suppression for hosts that own their existing notification loop.

Custom TypeScript transports expose a synchronous closedSignal so executor callbacks abort immediately on transport loss. The unused local startupTimeoutMs option is removed.

Release the reverse dependency closure together so registry builds share the new protocol and core types.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.7 (2026-09-26)

### Fixes

#### Per-turn OpenAI service tier (Fast mode), and GPT-6 Sol and Luna

A caller can now choose the OpenAI service tier for a turn.
`StartTurnRequest::service_tier_override` (for example `"priority"` for Fast
mode) is carried to every inference round of the turn as
`RuntimeHints::service_tier`. The OpenAI Responses provider sends it as the
top-level `service_tier` request field only on the OpenAI profile; OpenRouter,
xAI, and Fireworks never receive it. `None` keeps the provider default.

The tier OpenAI reports it actually served (`response.service_tier`) is recorded
on `TokenUsage::service_tier`, so a biller can tell a request that ran fast from
one that was downgraded to `"default"` under load. Both new fields are optional
and default to absent when older payloads are deserialized.

`StartTurnRequest` gains a public field, so code that builds it with a struct
literal must add `service_tier_override: None`.

The OpenAI/Codex catalog adds `gpt-6-sol` (efforts `low` through `max`, like
`gpt-6-astra`) and `gpt-6-luna` (efforts `low` through `max`, like
`gpt-5.6-luna`), so per-turn reasoning validation accepts them.

## 0.1.6 (2026-07-21)

### Fixes

#### Consume the portable supervised Claude Code SDK release

Depend on `claude-code-sdk-rust` 0.4.1 so published Roder consumers receive the
same cancellation-safe subprocess cleanup API as the repository build, without
relying on a workspace-local Cargo patch.

## 0.1.5 (2026-07-21)

### Fixes

#### Add bounded lifecycle recovery, cleanup proof, and shutdown diagnostics

Roder now persists redacted per-turn lifecycle records, reconciles interrupted
turns after restart, and reports bounded cleanup ownership rather than treating
an aborted runtime task as proof that provider work was reaped. Local process
tasks drain through graceful signal, forced kill, and reap; remote tasks use the
remote runner cancellation API; and the Claude Code provider uses a vendored SDK
cleanup path with offline real-child regression coverage.

The app-server adds lifecycle notifications, `runtime/drain`, and
`lifecycle/metrics`; the CLI and TUI expose durable recovery state. A shared
`[lifecycle]` configuration controls shutdown budgets, task policy, bounded
process diagnostics, and compatible legacy shutdown fallbacks.

## 0.1.4 (2026-06-27)

### Features

#### Let the Claude Code provider use the "Claude in Chrome" browser tools

The `claude-code` provider can now register against the local Claude Code
"Claude in Chrome" (CFC) integration so the model can drive the user's real
browser through the CLI's `mcp__claude-in-chrome__*` tools.

When enabled, the provider spawns `claude` with `CLAUDE_CODE_ENABLE_CFC=1` (so
the CLI wires its browser MCP server even in the SDK's headless/streaming mode)
and blanks `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN` for that child so the CLI
uses claude.ai subscription auth — a prerequisite for the Chrome integration to
connect. It pre-authorizes the browser tools through the SDK `can_use_tool`
callback (all other unmapped CLI tools stay denied) and surfaces the
CLI-executed browser tool calls as hosted tool calls
(`HostedToolCallStarted`/`HostedToolCallCompleted`) so they show in the UI
without the runtime trying to re-run a tool it never registered.

Enablement resolves from `ClaudeCodeConfig::enable_claude_in_chrome`, then the
`RODER_CLAUDE_CODE_ENABLE_CHROME`/`CLAUDE_CODE_ENABLE_CHROME` env vars, then
auto-detection of a paired/enabled Chrome extension in the local Claude Code
config. No `claude-agent-sdk` change was required.

## 0.1.3 (2026-06-24)

### Fixes

#### Lock in `request_user_input` support on the Claude Code path

Add the regression coverage the changelog already promised but that was missing
from the test suite. The new `tool_loop` test proves the interactive survey tool
is fully wired on the Claude Code path: it is advertised to the CLI as
`mcp__roder__request_user_input`, the `can_use_tool` callback pre-authorizes it,
and calling it routes through Roder's `TurnToolExecutor` with the nested
`questions` payload preserved through input repair (so the survey reaches the
runtime tool intact instead of being flattened by `retain_schema_properties`).
The runtime executor blocks the call until the client answers and returns the
resolved answers to the model.

#### Fix only the first claude-code tool call rendering in the TUI

The claude-code provider derived each in-process tool-call id solely from the
tool name (`claude-code-<Tool>`), so every later invocation of the same tool
(e.g. repeated `Bash`/`Read` calls) reused one id. The TUI and runtime key
tool-call rows by id, which collapsed all subsequent calls into the first
row, making it look like only the first tool call ever ran. Tool-call ids are
now made unique per invocation via a process-global counter
(`claude-code-<Tool>-<seq>`), so each call renders as its own row.

## 0.1.2 (2026-06-22)

### Features

#### Support hosted web search/fetch and survey prompts in the Claude Code provider

The Claude Code provider now enables the CLI's built-in `WebSearch` and
`WebFetch` tools whenever the turn requests hosted web search
(`RuntimeHints::hosted_web_search` is `cached` or `live`). Only those two
built-ins are turned on — every other built-in stays disabled so the model keeps
using the `mcp__roder__*` tools for filesystem/shell access — and the
`can_use_tool` callback pre-authorizes them. Because the CLI executes the web
tools itself, the provider surfaces them as hosted tool calls
(`HostedToolCallStarted`/`HostedToolCallCompleted`) instead of runtime tool
calls, so the activity is shown in the UI without the runtime trying to
re-execute a tool it never registered.

It also documents and adds regression coverage for `request_user_input`: the
survey tool is advertised as `mcp__roder__request_user_input` and routes through
Roder's `TurnToolExecutor`, which surfaces the survey questions to the client
and blocks the tool call until the user answers, returning the resolved answers
to the model.

### Fixes

#### Deliver image input to the Claude Code provider as real image blocks

The Claude Code provider advertised `image_input: true` but only ever sent the
transcript as a plain text string, so any image attached to a user message was
serialized via `format!("{item:?}")` — i.e. the base64 `data:` URL was dumped
into the prompt as text rather than passed to the model as an image.

The `claude-code-sdk-rust` SDK now exposes a `UserMessageInput` (text or a list
of `InputContentBlock`s, including base64/URL `ImageSource` image blocks) and
its streaming/query entrypoints accept it. The provider decodes each
`UserMessage` image data URL into a real image content block and only replays
the message text (without the raw base64 bytes) in the prompt, so multimodal
turns reach Claude correctly.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
