---
roder-ext-claude-code: minor
---

# Support hosted web search/fetch and survey prompts in the Claude Code provider

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
