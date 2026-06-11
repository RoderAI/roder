# Roder Cursor Provider

Roder exposes Cursor Composer as a first-class inference provider id:

```text
cursor
```

Cursor exposes Composer alongside frontier models it proxies through AgentService. The catalog ships these descriptors:

```text
cursor/composer-2.5
cursor/claude-opus-4-8
cursor/claude-sonnet-4-6
cursor/gpt-5.5
cursor/gemini-3.1-pro-preview
cursor/grok-4.3
```

Each proxied model reuses the same id as its native provider, so the model string sent to AgentService matches the underlying model.

`cursor/claude-opus-4-8` advertises configurable reasoning effort (`low`, `medium`, `high`, `xhigh`, `max`; default `high`), matching the native Anthropic Opus catalog entry. Note: the AgentService protobuf encoding does not yet forward the selected effort over the wire — that requires a captured Cursor model-options fixture. The other proxied models do not expose effort options.

## API Key Setup

Cursor support uses a Cursor User API Key. Configure it with either environment variable:

```sh
export CURSOR_API_KEY="..."
export RODER_CURSOR_API_KEY="..."
```

Or store it in Roder config:

```toml
[providers.cursor]
api_key_env = "CURSOR_API_KEY"
```

Raw Cursor keys and exchanged access tokens must stay out of checked-in files. Provider metadata only reports whether auth is configured.

## Transport

Roder does not call hosted OpenAI-compatible Cursor endpoints. Live probing showed these are not the provider path:

```text
https://api.cursor.com/v1/chat/completions
https://api.cursor.com/v1/responses
```

The native provider exchanges the Cursor API key through Cursor's key-exchange endpoint, then calls Cursor AgentService directly:

```text
POST https://agentn.global.api5.cursor.sh/agent.v1.AgentService/Run
content-type: application/connect+proto
```

This path uses HTTP/2 Connect/protobuf frames and does not execute `cursor-agent` at inference runtime. The Cursor CLI, `@cursor/sdk`, and StandardAgents-style OpenAI-compatible bridges are references for discovery and future compatibility work, not required runtime dependencies.

## Context

Cursor AgentService expects a serialized request-context frame. Roder builds this natively from repo-owned context such as:

```text
AGENTS.md
.agents/skills/*/SKILL.md
roadmap/68-roder-cursor-api-provider.md
roadmap/STATUS.md
README.md
Cargo.toml
```

The provider keeps the older discovery-trace context path behind `RODER_CURSOR_ALLOW_DISCOVERY_CONTEXT=1` for diagnostics only. Normal provider inference must not rely on a captured Cursor trace.

## Supported Surface

The first provider slice supports streaming text turns, model listing for the catalog's Cursor models, and Cursor-native read-file tool requests. Token usage is estimated from streamed prompt/output text when Cursor does not return official usage fields through this AgentService path; streamed thinking text is counted separately in provider metadata so the TUI can render the `thinking` token summary.

Cursor AgentService is tool-capable. A traced `cursor-agent --print --model composer-2.5` run produced a Cursor-native `readToolCall`, executed the file read locally, and wrote the tool result back into the same HTTP/2 AgentService stream. Roder now decodes the observed Cursor-native `readToolCall` response shape into canonical `read_file` tool calls, routes execution through Roder's tool registry and policy system, and replays tool call/result context into the next Cursor model round.

Unsupported request features fail before the prompt is sent to Cursor:

```text
structured response_format
background Responses jobs
audio output
logprobs
n > 1
```

### Cursor-native tool mapping

Roder decodes Cursor's `agent.v1.ClientSideToolV2` tool-call frames and maps them to canonical Roder tools (field numbers from the Cursor app bundle's `agent.v1` protobuf schema):

| Cursor `tool` oneof | Cursor args | Roder tool | Roder args |
|---|---|---|---|
| `read_tool_call` (8) | `ReadToolArgs { path 1, offset 2, limit 3 }` | `read_file` | `{ path, offset?, limit? }` |
| `edit_tool_call` (12) | `EditArgs { path 1, stream_content 6 }` | `write_file` | `{ path, content }` |
| `shell_tool_call` (1) | `ShellArgs { command 1, working_directory 2 }` | `shell` | `{ command, workdir? }` |
| `grep_tool_call` (5) | `GrepArgs { pattern 1, path 2 }` | `grep` | `{ query, path? }` |
| `glob_tool_call` (4) | `GlobToolArgs { glob_pattern 2 }` | `glob` | `{ pattern }` |

Two request requirements were discovered as prerequisites for the agentic tool loop:

- **Agent mode.** `UserMessage.mode` (field 4) must be `agent.v1.AgentMode.AGENT_MODE_AGENT` (1). Sending `AGENT_MODE_ASK` (2) runs the model read-only, so it injects `<system_reminder>Ask mode is active</system_reminder>` and refuses edits.
- **Tool-call ids.** Live Cursor tool ids are Anthropic-style `toolu_...`; the decoder accepts both `tool_` and `toolu_` prefixes.

Any unmapped Cursor-native tool is surfaced as a `cursor_unsupported_tool` call so the frame can be captured (see below) and decoded.

### MCP / client-advertised tools (not surfaced by Cursor)

`AgentRunRequest.mcp_tools` (field 4) carries `agent.v1.McpToolDefinition`s the client can advertise. Roder can map its own `ToolSpec`s into this field (`name 1`, `description 2`, `input_schema 3` as a `google.protobuf.Value`; see `encode_mcp_tools` and `proto/agent_v1.proto`), but **this does not expand a Cursor model's tool surface**.

A live experiment (`cursor/claude-opus-4-8` with `RODER_CURSOR_CAPTURE_FRAMES`) advertised a Roder-only tool (`update_plan`) and the model reported it only had Cursor's native tools (`TodoWrite`) — no `update_plan`, and the response stream contained no `GetMcpTools`/`McpTool`/`tool_not_found` frames. Cursor controls the model's tool set server-side; `McpToolDefinition` entries without a registered `provider_identifier` (i.e. a Cursor-hosted MCP server) are ignored. The turn was unaffected (no regression).

Consequently Roder's tools reach Cursor models **only** through the native exec mapping above (read/write/shell/grep/glob), not through `mcp_tools`. Advertising is therefore disabled by default. The encoder, schema (`proto/agent_v1.proto`), and tests are kept for future work (provider registration, or a client-exec channel for MCP). Set `RODER_CURSOR_ADVERTISE_MCP_TOOLS=1` to send the definitions anyway (harmless, currently a no-op on Cursor's side).

#### MCP server registration (investigation)

Bundle analysis (`cursor-agent-worker/dist/main.js`, `agent.v1`) shows the real registration path is **`AgentRunRequest.mcp_file_system_options` (field 6)**, not `mcp_tools` (field 4):

```
McpFileSystemOptions { enabled 1, workspace_project_dir 2, mcp_descriptors 3: repeated McpDescriptor }
McpDescriptor { server_name 1, server_identifier 2, folder_path 3, server_use_instructions 4,
                tools 5: repeated McpToolDescriptor, plugin 7, marketplace 8, ... }
McpToolDescriptor { tool_name 1, definition_path 2, description 3, input_schema 4: google.protobuf.Value }
```

Findings:

- The model's `ToolCall` oneof has **62 server-defined slots**; the surface is fixed server-side. The only MCP bridges are `mcp_tool_call` (15, invoke), `get_mcp_tools_tool_call` (44, discover), `list_mcp_resources_tool_call` (20) and `read_mcp_resource_tool_call` (21).
- `McpDescriptor.server_identifier` is the `provider_identifier` that `McpArgs` (an `mcp_tool_call`) must reference — which is exactly what our field-4 advertisement lacked, explaining why the model never saw `update_plan`.
- The `folder_path` / `definition_path` / "file system options" naming indicates Cursor is a **filesystem-based MCP host**: it reads tool definitions from files on disk, and `.cursor/mcp.json` `mcpServers` is the IDE-managed config that populates these descriptors.

**Resolved (bundle analysis, conclusive — no live capture required):** MCP tool *invocation is executed server-side by Cursor, never routed back to an AgentService client.** The decisive evidence is an asymmetry in the `agent.v1` schema:

- MCP **resource/state** ops have explicit client-exec types — `ReadMcpResourceExecArgs`/`…ExecResult`, `ListMcpResourcesExecArgs`/`…ExecResult`, `McpStateExecArgs`/`…ExecResult` — so Cursor *does* dispatch those back to the client over the exec channel.
- MCP **tool invocation** has **no** `*Exec*` type. `mcp_tool_call` (15) → `McpArgs`/`McpToolResult` is not an exec-serviceable op, so the client never runs it.

Instead, MCP servers are registered with Cursor's **backend** via the separate `aiserver.v1` service (`SetMcpConfig`, `GetMcpConfig`, `GetAvailableMcpServers`, `StoreMcpOAuthToken`, `MarkMcpServersSeen`, …), and the AgentService backend executes an `mcp_tool_call` against the server that `provider_identifier`/`server_identifier` names. The agent worker itself contains no MCP client/transport machinery (no stdio/SSE MCP transport, no `callTool`); its `child_process`/`http2`/`net` usage is only for the worker's own AgentService connection.

**Conclusion — not viable for Roder's in-process tools.** Because tool invocation is server-side and bound to a backend-registered `server_identifier`, an AgentService client like Roder cannot make a Cursor model call Roder's tool registry via MCP — neither by advertising `mcp_tools` (field 4, ignored) nor by sending `mcp_file_system_options` descriptors (field 6, which only name servers Cursor's backend already knows how to reach). The only way a Cursor model could reach Roder over MCP would be to run a real, separately-registered MCP server process that Cursor's backend connects to — which would not route through Roder's executor/policy and so defeats the purpose. **Roder's tools therefore reach Cursor models only through the native exec mapping (read/write/shell/grep/glob), which Cursor *does* round-trip to the client.** The `mcp_tools` encoder + schema are retained for reference but stay disabled by default (`RODER_CURSOR_ADVERTISE_MCP_TOOLS=1` to send anyway; a no-op on Cursor's side). The `*ExecArgs`/`*ExecResult` naming for `McpState`/`ListMcpResources`/`ReadMcpResource` hints at a client-exec path for *some* MCP ops, but the tool-invocation path is unconfirmed. Recommended next experiment: capture a live Cursor-IDE `AgentService/Run` turn with a real `.cursor/mcp.json` server configured, to learn (a) the exact `mcp_file_system_options` payload the IDE sends and (b) whether `mcp_tool_call` arrives on the exec channel for the client to execute. The schema for all of the above is recorded in `proto/agent_v1.proto`.

### Image input

The Cursor provider advertises `image_input: true` and uploads images inline (field numbers sourced from the Cursor app bundle's `agent.v1` protobuf schema, same as the tool mapping above):

- **Current message.** Images on the latest `UserMessage` are encoded into `UserMessage.selected_context` (field 3) as `agent.v1.SelectedContext.selected_images` (field 1), each a `SelectedImage { mime_type 7, data 8 }` carrying the raw decoded bytes.
- **History.** Images on prior user turns are replayed as `ConversationHistoryUserContent.image` (field 2) → `ConversationHistoryImageContent { data 1 (base64 string), mime_type 2 }`.
- **Capability flag.** When any inline image is present, `AgentRunRequest.client_supports_inline_images` (field 19) is set so Cursor accepts the inline bytes.

Roder `InputImage`s are `data:<mime>;base64,<payload>` URLs; non-base64 / remote-URL images are skipped because Cursor's inline path needs raw bytes. The Cursor catalog models advertise `supports_images: true`.

> **Known limitation (in progress):** tool *results* are currently replayed as flattened prompt text in the next round, and each round opens a fresh Cursor conversation. Cursor's agent does not treat that as a native tool result, so multi-step tool loops (e.g. read-then-edit) can re-issue the same tool call. Completing the loop requires sending results via `ConversationAction.resume_action` / `UserMessageAction.conversation_history` against a stable `conversation_id`.

## Model listing (live + on-disk cache)

`list_models` keeps the model set up to date from Cursor's own model picker RPC, then caches the merged result on disk (same pattern as the OpenAI/OpenCode/Poolside providers):

- **RPC:** `POST {backend}/aiserver.v1.AiService/AvailableModels` (Connect unary). Use `content-type: application/json` with body `{}` (the JSON Connect codec; `application/connect+proto` returns HTTP 415 for unary). Auth is the same exchanged access token + `connect-protocol-version: 1`, `x-cursor-client-type: cli`, `x-cursor-client-version`, `x-ghost-mode: true` headers as `AgentService/Run`. Backend host is the API key exchange host (`api2.cursor.sh`), not the AgentService host.
- **Response:** `{ models: [ { serverModelName, clientDisplayName, supportsAgent, supportsImages, supportsThinking, tooltipData{ markdownContent } } ] }`.
- **Important divergence:** the picker returns effort/fast variant ids (`claude-opus-4-8-thinking-high-fast`, `composer-2.5-fast`) that are **not** the ids `AgentService/Run` accepts. Run accepts the bare ids in Roder's curated catalog (`claude-opus-4-8`). So Roder reduces each picker id to its base form (strips trailing effort / `-thinking` / `-fast`) and **merges** it into the curated catalog rather than using picker ids verbatim: curated entries keep their hand-tuned context window + reasoning options; genuinely new base ids are appended with a cleaned display name and a context window parsed from the tooltip. Meta ids (`auto`/`default`) and namespaced ids (`accounts/.../...`) are skipped.

Caching: `~/.roder/models-cache.json` (override with `RODER_MODELS_CACHE_PATH`), 6h TTL (`RODER_MODELS_CACHE_TTL_SECONDS`), background refresh on staleness, force a refresh with `RODER_MODELS_REFRESH=1`. The live call only runs when Cursor auth is configured; otherwise the static catalog (`models_for_provider`) is returned. Any auth/network/HTTP error falls back to the static catalog, so model listing never hard-depends on the live call.

## Capturing Cursor-native tool frames

Mapping new Cursor-native tool calls (`edit`, `shell`, `search`, MCP) requires the real on-the-wire protobuf bytes. Set `RODER_CURSOR_CAPTURE_FRAMES` to a writable file path and every raw AgentService frame — outbound request frames (`"dir":"send"`) and inbound response payloads (`"dir":"recv"`) — is appended as hex JSONL. This is a diagnostic; with the variable unset there is zero overhead and no behavior change.

```sh
RODER_CURSOR_CAPTURE_FRAMES=/tmp/cursor-frames.jsonl \
  ./bin/roder
```

Then, with `cursor/claude-opus-4-8` (or any Cursor model) active, give a direct imperative edit instruction (for example, "Edit AGENTS.md and append a line that says hello"). Cursor's native agent attempts its native edit tool; the raw frame is recorded even though Roder currently decodes it as `cursor_unsupported_tool`. Each captured `"dir":"recv"` line that carries a tool-call payload is the fixture used to extend `decode_cursor_tool_call` in `crates/roder-ext-cursor/src/proto.rs`.

## Optional Overrides

Use endpoint overrides only for diagnostics against a compatible Cursor deployment:

```sh
export RODER_CURSOR_AGENT_SERVICE_URL="https://agentn.global.api5.cursor.sh"
export RODER_CURSOR_BACKEND_BASE_URL="https://api2.cursor.sh"
```

Local-development access-token overrides are accepted for diagnostics:

```sh
export CURSOR_ACCESS_TOKEN="..."
export CURSOR_AUTH_TOKEN="..."
```

Do not use access-token overrides as production configuration; prefer Cursor API-key exchange.

## Live Checks

Normal tests do not require Cursor credentials. Live checks are opt in:

```sh
cargo test -p roder-ext-cursor

RODER_CURSOR_LIVE=1 \
CURSOR_API_KEY="..." \
cargo test -p roder-ext-cursor live_cursor_composer_25 -- --ignored --nocapture
```

The live test covers both a proof-token response and a normal short-story prompt through direct AgentService API calls.
