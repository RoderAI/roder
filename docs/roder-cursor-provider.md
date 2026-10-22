# Roder Cursor Provider

Roder exposes Cursor Composer as a first-class inference provider id:

```text
cursor
```

Cursor exposes Composer alongside frontier models it proxies through AgentService. The catalog ships these descriptors:

```text
cursor/composer-2.5
cursor/composer-2.5-fast
cursor/claude-opus-4-8
cursor/claude-sonnet-4-6
cursor/gemini-3.1-pro
cursor/gemini-4-argon
cursor/grok-4.6
```

Fast variants (`*-fast`) are separate catalog ids. AgentService/Run accepts the bare model id (`composer-2.5`, `claude-opus-4-8`) with a `fast=true` requested-model param; Roder maps `composer-2.5-fast` automatically when encoding the protobuf request.

Each proxied model reuses the same id as its native provider, so the model string sent to AgentService matches the underlying model.

`cursor/claude-opus-4-8` advertises configurable reasoning effort (`low`, `medium`, `high`, `xhigh`, `max`; default `high`), matching the native Anthropic Opus catalog entry. The selected effort is forwarded in `requested_model.params`. Reasoning options for other models come from the merged catalog.

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

The native bidirectional runtime supports streaming text and reasoning, model listing, multi-turn history, and policy-approved read/write/delete/shell/search/directory-list operations. Results come from Roder's executor, including remote workspace output, structured file contents, errors, and shell exit codes. The provider never retries a denied operation against its own filesystem.

Usage uses Cursor's turn-end input/output token fields when available and falls back to text estimates for missing fields. Provider metadata marks estimated totals. A dropped stream or a stalled turn raises an error instead of a successful completion; heartbeats stop when the consumer drops the stream.

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
| `read_tool_call` (8) | `ReadToolArgs { path 1, offset 2, limit 3 }` | `read_file` | `{ path, start_line?, limit? }` |
| `edit_tool_call` (12) | `EditArgs { path 1, stream_content 6 }` | `write_file` | `{ path, content }` |
| `shell_tool_call` (1) | `ShellArgs { command 1, working_directory 2 }` | `shell` | `{ command, workdir? }` |
| `grep_tool_call` (5) | `GrepArgs { pattern 1, path 2 }` | `grep` | `{ query, path? }` |
| `glob_tool_call` (4) | `GlobToolArgs { glob_pattern 2 }` | `glob` | `{ pattern }` |

Two request requirements were discovered as prerequisites for the agentic tool loop:

- **Agent mode.** `UserMessage.mode` (field 4) must be `agent.v1.AgentMode.AGENT_MODE_AGENT` (1). Sending `AGENT_MODE_ASK` (2) runs the model read-only, so it injects `<system_reminder>Ask mode is active</system_reminder>` and refuses edits.
- **Tool-call ids.** Live Cursor tool ids are Anthropic-style `toolu_...`; the decoder accepts both `tool_` and `toolu_` prefixes.

Any unmapped Cursor-native tool is surfaced as a `cursor_unsupported_tool` call so the frame can be captured (see below) and decoded.

### MCP / client-advertised tools

Cursor Desktop 3.25.9's generated schema explicitly includes `ExecServerMessage.mcp_args` (field 11) and `ExecClientMessage.mcp_result` (field 11). Earlier documentation inferred that invocation could only execute on Cursor's backend because the message name lacked an `Exec` suffix. That inference was incorrect.

Roder decodes MCP argument maps as `google.protobuf.Value`, dispatches the exact tool name and arguments through its normal runtime executor, and returns text, structured content, and `is_error`. Definitions are advertised by default with `provider_identifier="roder"` and the underlying tool name. MCP state requests return the current runtime tool definitions. Live `composer-2.5` verification through the TUI exercised discovery and two `update_plan` calls, alongside native search, file creation, and read-back.

Diagnostics requests receive an explicit typed error because Roder has no diagnostics executor. Multiline grep and other unmapped exec variants also fail explicitly. Unsupported variants use `ExecClientControlMessage.throw` with the original request id; request/result field numbers cannot be assumed symmetric (the current Pi tool slots differ). No empty success is fabricated.

### Image input

The Cursor provider advertises `image_input: true` and uploads images inline (field numbers sourced from the Cursor app bundle's `agent.v1` protobuf schema, same as the tool mapping above):

- **Current message.** Images on the latest `UserMessage` are encoded into `UserMessage.selected_context` (field 3) as `agent.v1.SelectedContext.selected_images` (field 1), each a `SelectedImage { mime_type 7, data 8 }` carrying the raw decoded bytes.
- **History.** A native checkpoint retains prior image context. When importing Roder history into a fresh Cursor conversation, prior images are attached before the current message's images through `SelectedContext`.
- **Capability flag.** When any inline image is present, `AgentRunRequest.client_supports_inline_images` (field 19) is set so Cursor accepts the inline bytes.

Roder `InputImage`s are `data:<mime>;base64,<payload>` URLs; non-base64 / remote-URL images are skipped because Cursor's inline path needs raw bytes. The Cursor catalog models advertise `supports_images: true`.

Roder reuses a stable Cursor `conversation_id` for every Roder thread. Continuity also requires the native checkpoint: `AgentServerMessage.conversation_checkpoint_update` contains a `ConversationStateStructure` whose messages and turns reference blobs by id. Roder stores both the opaque checkpoint and KV blobs in `ProviderMetadata.cursorState`, sends the checkpoint and `pre_fetched_blobs` on the next request, and answers KV GETs with the actual bytes. A conversation id alone does not restore the prior turn.

Fresh conversations without a native checkpoint import the Roder transcript as explicit JSON reference context in the initial prompt. This also handles compaction summaries and provider handoffs; old context is cleared at a compaction boundary. The hosted Composer harness ignored the former field-7 history injection in live tests, so that request path has been removed. Native continuation uses checkpoint state without duplicating this imported context.

Low-level `InferenceEngine` integrators must retain and replay provider metadata along with the transcript. Roder's runtime does this automatically. Tool executors return raw structured output in `TurnToolOutcome.data`; native and MCP tool requests use the same executor and policy path. The provider uses one bidirectional transport for both text-only and tool-enabled requests.

## Model listing (live + on-disk cache)

`list_models` keeps the model set up to date from Cursor's own model picker RPC, then caches the merged result on disk (same pattern as the OpenAI/OpenCode/Poolside providers):

- **RPC:** `POST {backend}/aiserver.v1.AiService/AvailableModels` (Connect unary). Use `content-type: application/json` with body `{}` (the JSON Connect codec; `application/connect+proto` returns HTTP 415 for unary). Auth is the same exchanged access token + `connect-protocol-version: 1`, `x-cursor-client-type: cli`, `x-cursor-client-version`, `x-ghost-mode: true` headers as `AgentService/Run`. Backend host is the API key exchange host (`api2.cursor.sh`), not the AgentService host.
- **Response:** `{ models: [ { serverModelName, clientDisplayName, supportsAgent, supportsImages, supportsThinking, tooltipData{ markdownContent } } ] }`.
- **Important divergence:** the picker returns effort/fast variant ids (`claude-opus-4-8-thinking-high-fast`, `composer-2.5-fast`) that are **not** the ids `AgentService/Run` accepts as field `requested_model.model_id`. Run accepts the bare ids (`claude-opus-4-8`, `composer-2.5`) plus optional params such as `fast=true`. Roder therefore reduces picker ids to their base form when refreshing the catalog, and exposes supported fast variants (`composer-2.5-fast`) as first-class catalog ids that encode as bare id + `fast=true` on the wire.

Caching: `~/.roder/models-cache.json` (override with `RODER_MODELS_CACHE_PATH`), 6h TTL (`RODER_MODELS_CACHE_TTL_SECONDS`), background refresh on staleness, force a refresh with `RODER_MODELS_REFRESH=1`. The live call only runs when Cursor auth is configured; otherwise the static catalog (`models_for_provider`) is returned. Any auth/network/HTTP error falls back to the static catalog, so model listing never hard-depends on the live call.

## Capturing Cursor-native tool frames

Mapping new Cursor-native tool calls (`edit`, `shell`, `search`, MCP) requires the real on-the-wire protobuf bytes. Set `RODER_CURSOR_CAPTURE_FRAMES` to a writable file path and every raw AgentService frame — outbound request frames (`"dir":"send"`) and inbound response payloads (`"dir":"recv"`) — is appended as hex JSONL. This is a diagnostic; with the variable unset there is zero overhead and no behavior change.

```sh
RODER_CURSOR_CAPTURE_FRAMES=/tmp/cursor-frames.jsonl \
  ./bin/roder
```

Then, with `cursor/claude-opus-4-8` (or any Cursor model) active, give a direct imperative edit instruction (for example, "Edit AGENTS.md and append a line that says hello"). Cursor's native agent attempts its native edit tool; the raw frame is recorded even though Roder services the corresponding exec request. Each captured `"dir":"recv"` line that carries a tool-call payload is the fixture used to extend `decode_cursor_tool_call` in `crates/roder-ext-cursor/src/proto.rs`.

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

The continuity live suite verifies both checkpoint restoration and importing history without a checkpoint:

```sh
RODER_CURSOR_LIVE=1 mise exec -- cargo test -p roder-ext-cursor --test live_continuity -- --ignored
```

Retired Cursor models are excluded from fallback, cached, and live-discovered
lists. Cursor rejects `grok-4.3` as deprecated (use `grok-4.5` or a currently
listed newer Grok model); retired GPT-5.5 variants also stay hidden when present
in older caches. This filter is specific to Cursor and does not change other
providers' model availability.

Cursor's Gemini roster includes `gemini-4-argon` (Gemini 4 Argon, 1M context,
high effort by default) and canonical `gemini-3.1-pro` (1M context). The stale
`gemini-3.1-pro-preview` and `gemini-3.1-pro-preview-customtools` ids are excluded
from Cursor fallback, cached, and live-discovered lists. Other providers retain
their own model ids. Only Gemini 4 variants actually listed by Cursor are added.

Gemini 4 Argon uses `requested_model.params` with `reasoning_effort`
(`low`/`medium`/`high`). Do not send the generic `thinking`, `context`, `effort`,
or `fast` parameters for this registry-backed model. Both Argon and canonical
Gemini 3.1 Pro passed live AgentService inference on 2026-10-22.
