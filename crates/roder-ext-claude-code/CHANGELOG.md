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
