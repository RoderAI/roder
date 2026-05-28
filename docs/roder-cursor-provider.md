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

Additional Cursor-native tool variants such as search, shell, edits, and MCP require dedicated protobuf fixtures before they are mapped.

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
