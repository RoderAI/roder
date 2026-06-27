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
