# claude-code-sdk-rust

[![crates.io](https://img.shields.io/crates/v/claude-code-sdk-rust.svg)](https://crates.io/crates/claude-code-sdk-rust)
[![docs.rs](https://docs.rs/claude-code-sdk-rust/badge.svg)](https://docs.rs/claude-code-sdk-rust)
[![license](https://img.shields.io/crates/l/claude-code-sdk-rust.svg)](./LICENSE)

An async Rust SDK for the Claude Code CLI.

> **Crate name vs. import path:** this crate is published on crates.io as
> **`claude-code-sdk-rust`**, but its library import path is **`claude_agent_sdk`**.
> Add `claude-code-sdk-rust` to your `Cargo.toml` and write
> `use claude_agent_sdk::...;` in your code.

This crate provides a Tokio-native SDK for driving an authenticated local `claude` CLI process with typed options, message parsing, interactive sessions, control requests, SDK MCP servers, and local/session-store helpers. It is modeled after the Python `claude-agent-sdk` API while using Rust-native async, traits, and type-safe data structures.

## Status

This repository is an SDK implementation, not an official Anthropic package. It targets parity with the Python Claude Agent SDK public behavior and is verified by the local test suite.

Verification:

```bash
cargo test
cargo check --features otel
```

The normal test suite avoids paid/authenticated CLI calls. The ignored `e2e_cli` smoke test can be run separately when the Claude CLI is authenticated.

## Requirements

- Rust 1.70+
- Tokio runtime
- Claude Code CLI installed and authenticated
- `claude` available on `PATH`, or set via `ClaudeAgentOptions::builder().cli_path(...)`

## Installation

Add the crate from crates.io:

```toml
[dependencies]
claude-code-sdk-rust = "0.4"
tokio = { version = "1", features = ["full"] }
```

Then import it through its library path, `claude_agent_sdk`:

```rust
use claude_agent_sdk::query;
```

Optional OpenTelemetry propagation support:

```toml
[dependencies]
claude-code-sdk-rust = { version = "0.4", features = ["otel"] }
```

Or track the development branch directly from Git:

```toml
[dependencies]
claude-code-sdk-rust = { git = "https://github.com/PandelisZ/claude-agent-sdk-rust" }
```

## Quick start

### One-shot query

```rust
use claude_code_sdk_rust::query;

#[tokio::main]
async fn main() -> claude_code_sdk_rust::Result<()> {
    let result = query("Explain ownership in Rust in two sentences.", None).await?;
    println!("{}", result.content);
    Ok(())
}
```

### Full message sequence

```rust
use claude_code_sdk_rust::query_messages;

#[tokio::main]
async fn main() -> claude_code_sdk_rust::Result<()> {
    let messages = query_messages("List the files you would inspect first.", None).await?;

    for message in messages {
        println!("{message:?}");
    }

    Ok(())
}
```

### Streamed one-shot input

```rust
use claude_code_sdk_rust::query_stream_messages;

#[tokio::main]
async fn main() -> claude_code_sdk_rust::Result<()> {
    let stream = futures::stream::iter(vec![
        serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": "First prompt frame"}
        }),
        serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": "Second prompt frame"}
        }),
    ]);

    let messages = query_stream_messages(stream, None).await?;
    println!("received {} messages", messages.len());
    Ok(())
}
```

## Interactive client

The interactive client mirrors the Python SDK's explicit connection model: create the client, call `connect()`, send prompts or control requests, receive messages, then `disconnect()` or `close()`.

```rust
use claude_code_sdk_rust::{ClaudeAgentClient, ClaudeAgentOptions, PermissionMode};

#[tokio::main]
async fn main() -> claude_code_sdk_rust::Result<()> {
    let options = ClaudeAgentOptions::builder()
        .model("claude-sonnet-4-20250514")
        .permission_mode(PermissionMode::AcceptEdits)
        .build();

    let mut client = ClaudeAgentClient::new(options)?;
    client.connect().await?;

    client.query("Inspect this repository and summarize the test strategy.").await?;
    let messages = client.receive_response().await?;

    for message in messages {
        println!("{message:?}");
    }

    client.disconnect().await?;
    Ok(())
}
```

Convenience connection helpers are also available:

```rust
client.connect_with_prompt("Start by reading Cargo.toml").await?;
client.connect_with_stream(prompt_stream).await?;
```

## Streaming responses

```rust
use claude_code_sdk_rust::{ClaudeAgentClient, ClaudeAgentOptions, StreamEvent};

#[tokio::main]
async fn main() -> claude_code_sdk_rust::Result<()> {
    let mut client = ClaudeAgentClient::new(ClaudeAgentOptions::default())?;
    client.connect().await?;

    let mut events = client.stream_message("Write a short haiku about compilers.").await?;
    while let Some(event) = events.recv().await {
        match event {
            StreamEvent::ContentChunk(text) => print!("{text}"),
            StreamEvent::Complete(response) => println!("\nfinished: {:?}", response.stop_reason),
            StreamEvent::Error(error) => eprintln!("error: {error}"),
            _ => {}
        }
    }

    client.disconnect().await?;
    Ok(())
}
```

## Configuration

Use `ClaudeAgentOptions::builder()` for ergonomic configuration.

```rust
use claude_code_sdk_rust::{
    ClaudeAgentOptions, MCPServerConfig, PermissionMode, SettingSource, SkillsConfig,
};
use std::collections::HashMap;

let mut env = HashMap::new();
env.insert("RUST_LOG".to_string(), "info".to_string());

let options = ClaudeAgentOptions::builder()
    .cwd("/path/to/project")
    .model("claude-sonnet-4-20250514")
    .system_prompt("You are a careful Rust coding agent.")
    .allowed_tools(vec!["Read".into(), "Edit".into(), "Bash".into()])
    .permission_mode(PermissionMode::AskOnFirstUse)
    .setting_sources(vec![SettingSource::User, SettingSource::Project])
    .skills(SkillsConfig::All)
    .env(env)
    .mcp_server(
        "docs",
        MCPServerConfig::Stdio {
            command: "docs-mcp".into(),
            args: Some(vec!["--stdio".into()]),
            env: None,
        },
    )
    .build();
```

Supported option areas include:

- CLI path, working directory, environment, extra CLI args
- model, fallback model, betas, thinking config, task budgets
- system prompt presets/files, dynamic prompt controls, skills config
- allowed/disallowed tools and permission mode
- `can_use_tool` callback support for streaming/client mode
- hooks and hook event inclusion
- stdio, SSE, HTTP, raw/path MCP server config
- in-process SDK MCP servers
- sandbox settings
- session resume, continue, fork, and session-store mirroring
- stderr callback
- optional OpenTelemetry context propagation with the `otel` feature

## SDK MCP servers

The crate can host in-process SDK MCP tools and bridge them through Claude Code control requests.

```rust
use claude_code_sdk_rust::{create_sdk_mcp_server, tool, ClaudeAgentOptions, MCPContent};

let server = create_sdk_mcp_server(
    "local-tools",
    vec![tool(
        "greet",
        "Greet a user",
        serde_json::json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        }),
        |input| {
            let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("there");
            Ok(vec![MCPContent::Text { text: format!("Hello, {name}!") }])
        },
    )],
);

let options = ClaudeAgentOptions::builder()
    .sdk_mcp_server("local-tools", server)
    .build();
```

Use `create_sdk_mcp_server_with_version` and `tool_with_annotations` when the CLI needs server version metadata or MCP tool annotations.

## Permission callbacks

```rust
use claude_code_sdk_rust::{ClaudeAgentOptions, PermissionResult};

let options = ClaudeAgentOptions::builder()
    .can_use_tool(|tool_name, input, _context| async move {
        if tool_name == "Bash" {
            return Ok(PermissionResult::deny("Bash is disabled in this run"));
        }
        println!("allowing {tool_name} with input {input:?}");
        Ok(PermissionResult::allow())
    })
    .build();
```

`can_use_tool` requires streaming/client mode. String-based one-shot `query_messages(...)` rejects this callback because there is no live bidirectional control channel after spawning the subprocess.

## Hooks

Hooks are represented as callback matchers grouped by event name.

```rust
use claude_code_sdk_rust::{ClaudeAgentOptions, HookCallback, HookMatcher};

let callback = HookCallback::new(|input, _tool_use_id, _context| async move {
    println!("hook input: {input:?}");
    Ok(serde_json::json!({"continue": true}))
});

let options = ClaudeAgentOptions::builder()
    .hook("PreToolUse", HookMatcher::new(callback).matcher("Edit"))
    .include_hook_events(true)
    .build();
```

## Sessions

Local Claude transcript helpers are available from the crate root and from `claude_code_sdk_rust::sessions`.

```rust
use claude_code_sdk_rust::{get_session_info, get_session_messages, list_sessions};

#[tokio::main]
async fn main() -> claude_code_sdk_rust::Result<()> {
    let sessions = list_sessions(None).await?;

    for session in sessions {
        println!("{} {:?}", session.id, session.title);
    }

    let info = get_session_info("session-uuid", None).await?;
    let messages = get_session_messages("session-uuid", None).await?;

    println!("{info:?}");
    println!("{} messages", messages.len());
    Ok(())
}
```

Session helpers include:

- list, inspect, page, and delete local sessions
- list and read subagent transcripts
- rename and tag sessions by appending metadata events
- fork sessions with remapped IDs
- import local transcripts into a custom session store
- store-backed list/read/mutate/fork helpers

## Custom session stores

Implement `SessionStore` for external persistence. Stores can opt into session listing by overriding `supports_list_sessions()` and `list_sessions(...)`.

```rust
use async_trait::async_trait;
use claude_code_sdk_rust::{
    Result, SessionKey, SessionListSubkeysKey, SessionStore, SessionStoreEntry,
    SessionStoreListEntry,
};

struct MyStore;

#[async_trait]
impl SessionStore for MyStore {
    async fn append(&self, key: SessionKey, entries: Vec<SessionStoreEntry>) -> Result<()> {
        println!("append {} entries to {key:?}", entries.len());
        Ok(())
    }

    async fn load(&self, key: SessionKey) -> Result<Option<Vec<SessionStoreEntry>>> {
        println!("load {key:?}");
        Ok(None)
    }

    async fn delete(&self, key: SessionKey) -> Result<()> {
        println!("delete {key:?}");
        Ok(())
    }

    async fn list_subkeys(&self, key: SessionListSubkeysKey) -> Result<Vec<String>> {
        println!("list subkeys for {key:?}");
        Ok(Vec::new())
    }

    fn supports_list_sessions(&self) -> bool {
        true
    }

    async fn list_sessions(&self, _project_key: &str) -> Result<Vec<SessionStoreListEntry>> {
        Ok(Vec::new())
    }
}
```

## Error handling

Most functions return `claude_code_sdk_rust::Result<T>`, an alias for `Result<T, ClaudeSDKError>`.

Main error variants include:

- `CLIConnectionError`
- `CLINotFoundError`
- `ProcessError`
- `CLIJSONDecodeError`
- `MessageParseError`
- generic SDK/configuration errors

## Testing

Run the standard suite:

```bash
cargo test
```

Run feature verification:

```bash
cargo check --features otel
```

Run the ignored real CLI smoke test only when authenticated and prepared for possible API usage:

```bash
cargo test --test e2e_cli -- --ignored
```

## Crate layout

- `client` - interactive client
- `query` - one-shot query helpers
- `types` - public SDK data types
- `options` - builder API
- `mcp` - SDK MCP server/tool helpers
- `session_store` - custom store trait and in-memory store
- `sessions` - local and store-backed session helpers
- `internal` - CLI args, transport, protocol, control, parsing, mirroring

## License

MIT
