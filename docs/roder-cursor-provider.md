# Roder Cursor Provider

Roder exposes Cursor Composer as a first-class inference provider id:

```text
cursor
```

The initial model descriptor is:

```text
cursor/composer-2.5
```

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

The first provider slice supports streaming text turns, fallback model listing for `composer-2.5`, and Cursor-native read-file tool requests. Token usage is estimated from streamed prompt/output text when Cursor does not return official usage fields through this AgentService path; streamed thinking text is counted separately in provider metadata so the TUI can render the `thinking` token summary.

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
