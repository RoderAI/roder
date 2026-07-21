# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
(pre-1.0: breaking changes bump the minor version).

## [0.4.0] - 2026-06-22

### Added

- Image (multimodal) prompt input. New public types in `types::messages`
  (re-exported at the crate root):
  - `UserMessageInput` — either `Text(String)` or `Blocks(Vec<InputContentBlock>)`.
  - `InputContentBlock` — `Text { text }` or `Image { source }`.
  - `ImageSource` — `Base64 { media_type, data }` or `Url { url }`, with
    `ImageSource::from_data_url` to parse `data:<mime>;base64,<payload>` URLs.
- A user message's `content` is now serialized as a JSON string for text-only
  prompts, or as a content-block array (text + images) for multimodal prompts.

### Changed

- **Breaking:** the streaming/query entrypoints (`spawn_stream_message`,
  `stream_message`, `query`, `query_with_session_id`, `connect_with_prompt`,
  `send_message`) now accept `impl Into<UserMessageInput>` instead of
  `impl Into<String>`. `String` and `&str` callers are unaffected; callers
  passing other `Into<String>` types must convert to `String`/`UserMessageInput`.

## [0.3.1] - 2026-06-22

### Added

- End-to-end test coverage (all `#[ignore]`d; require an authenticated `claude`
  CLI and incur API usage):
  - `tests/e2e_tool_use.rs` — in-process SDK MCP tool dispatch, built-in Bash
    tool execution, `get_mcp_status` control round-trip, and interactive
    multi-turn context retention.
  - `tests/e2e_web_tools.rs` — built-in `WebSearch` and `WebFetch` tool
    invocation with live result round-tripping.

### Notes

- No library/runtime changes; this release only adds opt-in e2e tests, so it is
  API-compatible with `0.3.0`.

## [0.3.0] - 2026-06-22

Upstream parity pass against the Python `claude-agent-sdk` **v0.2.106**
(`anthropics/claude-agent-sdk-python`).

### Added

- `EffortLevel` enum (`low` / `medium` / `high` / `xhigh` / `max`) mirroring the
  upstream `EffortLevel` literal, serialized to the bare CLI string for
  `--effort`.
- `ServerToolName` enum for server-side tool names (`advisor`, `web_search`,
  `web_fetch`, `code_execution`, `bash_code_execution`,
  `text_editor_code_execution`, `tool_search_tool_regex`,
  `tool_search_tool_bm25`). Deserializes forward-compatibly: unknown names are
  preserved in `ServerToolName::Other` instead of failing to parse.
- Typed hook input/output payloads matching the Python type surface:
  `HookInput` (discriminated on `hook_event_name`), `BaseHookInput`, and the
  per-event `PreToolUseHookInput`, `PostToolUseHookInput`,
  `PostToolUseFailureHookInput`, `UserPromptSubmitHookInput`, `StopHookInput`,
  `SubagentStopHookInput`, `PreCompactHookInput`, `NotificationHookInput`,
  `SubagentStartHookInput`, `PermissionRequestHookInput`; plus hook-specific
  outputs `PostToolUseFailureHookSpecificOutput`,
  `NotificationHookSpecificOutput`, `SubagentStartHookSpecificOutput`, and
  `PermissionRequestHookSpecificOutput`. The hook callback itself stays generic
  over `serde_json::Value`.
- `task_updated` lifecycle system message: `TaskUpdatedMessage`,
  `TaskUpdatedStatus`, `TERMINAL_TASK_STATUSES`, and `is_terminal_task_status`.
  The parser derives `status` from `patch.status` and parses defensively.
- `ClaudeAgentClient::spawn_stream_message` convenience for fire-and-forget
  single-prompt streaming.

### Changed

- **Breaking:** `ClaudeAgentOptions::effort` is now `Option<EffortLevel>` (was
  `Option<String>`); the builder `effort(..)` takes an `EffortLevel`.
- **Breaking:** `ContentBlock::ServerToolUse::name` is now `ServerToolName` (was
  `String`).

## [0.2.0] - 2026-06

- Published to crates.io as `claude-code-sdk-rust` (library import path
  `claude_agent_sdk`).

## [0.1.1] - 2026-06

- Maintenance release.

## [0.1.0] - 2026-06

- Initial crates.io release: async Rust SDK for the Claude Code CLI with typed
  options, message parsing, interactive sessions, control requests, SDK MCP
  servers, and local/session-store helpers.

[0.3.0]: https://github.com/PandelisZ/claude-agent-sdk-rust/releases/tag/v0.3.0
[0.2.0]: https://crates.io/crates/claude-code-sdk-rust/0.2.0
[0.1.1]: https://crates.io/crates/claude-code-sdk-rust/0.1.1
[0.1.0]: https://crates.io/crates/claude-code-sdk-rust/0.1.0
