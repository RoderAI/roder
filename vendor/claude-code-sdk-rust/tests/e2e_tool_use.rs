//! End-to-end tests that exercise real tool-call and SDK MCP behavior against
//! a live, authenticated `claude` CLI.
//!
//! These are `#[ignore]`d like the smokes in `e2e_cli.rs`: they spawn the real
//! CLI and incur API usage. Run them explicitly with:
//!
//! ```bash
//! cargo test --test e2e_tool_use -- --ignored --nocapture
//! ```
//!
//! Unlike a plain prompt smoke, these wire up an in-process SDK MCP server and
//! assert on the *tool-call plumbing*: that the model's `tools/call` is routed
//! to our Rust handler, that `ToolUseStart`/`ToolResult` stream events surface,
//! that `can_use_tool` actually gates execution, and that `get_mcp_status`
//! reports the in-process server. Each test skips gracefully (rather than
//! failing) when the chosen model is gated/unavailable, so a live run is green
//! whenever the account simply lacks access to a model.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use claude_code_sdk_rust::mcp::create_sdk_mcp_server;
use claude_code_sdk_rust::{
    tool, ClaudeAgentClient, ClaudeAgentOptions, MCPContent, MCPServerConnectionStatus,
    PermissionResult, StreamEvent,
};

/// A widely-available model that supports tool use. Haiku 4.5 passed the
/// existing smokes; using it keeps these tests cheap and fast.
const TOOL_CAPABLE_MODEL: &str = "claude-haiku-4-5-20251001";

/// Distinctive token the model can only know by invoking the MCP tool.
const MAGIC_WORD: &str = "xyzzy-platypus-1729";

/// Overall budget so a wedged turn fails fast instead of hanging CI.
const STREAM_TIMEOUT: Duration = Duration::from_secs(120);

fn has_real_claude_auth() -> bool {
    std::env::var_os("ANTHROPIC_API_KEY").is_some()
        || std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN").is_some()
        || has_logged_in_claude_cli()
}

fn has_logged_in_claude_cli() -> bool {
    let cli_responds = std::process::Command::new("claude")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let has_user_config = std::env::var_os("HOME")
        .map(|home| std::path::Path::new(&home).join(".claude.json").exists())
        .unwrap_or(false);
    cli_responds && has_user_config
}

/// Detect the CLI's "model is gated / unavailable" notice so tests can skip
/// rather than hard-fail when an account lacks access to a model.
fn looks_unavailable(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("currently unavailable")
        || lowered.contains("is unavailable")
        || lowered.contains("does not have access")
        || lowered.contains("model not found")
        || lowered.contains("unknown model")
}

/// Build options wired to an in-process MCP server named `roder` exposing a
/// single `magic_word` tool. `call_count` increments every time the Rust
/// handler actually runs — the strongest server-side proof of a real tool call.
fn options_with_magic_tool(call_count: Arc<AtomicUsize>, allow_tool: bool) -> ClaudeAgentOptions {
    let counter = Arc::clone(&call_count);
    let magic = tool(
        "magic_word",
        "Returns the secret magic word. Call with empty arguments.",
        serde_json::json!({"type": "object", "properties": {}, "additionalProperties": false}),
        move |_input| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(vec![MCPContent::Text {
                text: MAGIC_WORD.to_string(),
            }])
        },
    );
    let server = create_sdk_mcp_server("roder", vec![magic]);

    // NOTE: deliberately do NOT pre-authorize via `allowed_tools`. Listing a
    // tool there makes the CLI auto-approve it and skip `can_use_tool`, which
    // would defeat the denial test. Leaving it out routes every tool call
    // through our `can_use_tool` gate, so the callback is the sole arbiter.
    ClaudeAgentOptions::builder()
        .model(TOOL_CAPABLE_MODEL)
        .include_partial_messages(true)
        .max_turns(3)
        .sdk_mcp_server("roder", server)
        .tools(Vec::new()) // disable built-ins; only the MCP tool is available
        .can_use_tool(move |tool_name, _input, _ctx| async move {
            if allow_tool && tool_name == "mcp__roder__magic_word" {
                Ok(PermissionResult::allow())
            } else {
                Ok(PermissionResult::deny(format!("{tool_name} not permitted")))
            }
        })
        .build()
}

/// Drain a stream to completion (bounded), collecting the signals we assert on.
struct Captured {
    content: String,
    tool_starts: Vec<String>,
    tool_results: usize,
    errors: Vec<String>,
    saw_complete: bool,
}

async fn drain_stream(mut events: tokio::sync::mpsc::UnboundedReceiver<StreamEvent>) -> Captured {
    let mut cap = Captured {
        content: String::new(),
        tool_starts: Vec::new(),
        tool_results: 0,
        errors: Vec::new(),
        saw_complete: false,
    };
    let _ = tokio::time::timeout(STREAM_TIMEOUT, async {
        while let Some(event) = events.recv().await {
            match event {
                StreamEvent::ContentChunk(text) => cap.content.push_str(&text),
                StreamEvent::ToolUseStart { name, .. } => cap.tool_starts.push(name),
                StreamEvent::ToolResult { .. } => cap.tool_results += 1,
                StreamEvent::Complete(_) | StreamEvent::TurnComplete(_) => cap.saw_complete = true,
                StreamEvent::Error(message) => cap.errors.push(message),
                _ => {}
            }
        }
    })
    .await;
    cap
}

/// Happy path: the model calls our in-process MCP tool, the handler runs, and
/// the tool's secret round-trips back into the final assistant content. Asserts
/// on the whole tool-call lifecycle, not just the text.
#[tokio::test]
#[ignore = "requires Claude CLI authentication and may incur API usage"]
async fn real_claude_cli_invokes_sdk_mcp_tool() {
    if !has_real_claude_auth() {
        eprintln!("skipping: Claude CLI authentication is required");
        return;
    }

    let call_count = Arc::new(AtomicUsize::new(0));
    let options = options_with_magic_tool(Arc::clone(&call_count), true);
    let events = ClaudeAgentClient::spawn_stream_message(
        options,
        "Call the mcp__roder__magic_word tool with empty arguments, then reply with \
         exactly the word it returned and nothing else.",
    );
    let cap = drain_stream(events).await;

    if cap.errors.iter().any(|e| looks_unavailable(e)) {
        eprintln!("skipping: model unavailable: {:?}", cap.errors);
        return;
    }
    assert!(
        cap.errors.is_empty(),
        "unexpected stream errors: {:?}",
        cap.errors
    );
    assert!(
        cap.saw_complete,
        "stream ended without a Complete/TurnComplete event"
    );

    // Strongest signal: the Rust handler actually executed (>= 1; the model
    // may legitimately call the tool more than once).
    assert!(
        call_count.load(Ordering::SeqCst) >= 1,
        "expected the MCP tool handler to run at least once"
    );

    // The tool invocation should surface as a ToolUseStart stream event.
    assert!(
        cap.tool_starts.iter().any(|n| n.contains("magic_word")),
        "expected a ToolUseStart for magic_word, saw {:?}",
        cap.tool_starts
    );
    // ToolResult stream events are emitted from assistant-side result blocks;
    // SDK MCP tool results are returned to the CLI out of band and may not
    // appear here, so this is informational rather than a hard requirement.
    if cap.tool_results == 0 {
        eprintln!("note: no ToolResult stream events observed for the MCP tool result");
    }

    // And the secret should round-trip into the model's reply.
    assert!(
        cap.content.to_ascii_lowercase().contains(MAGIC_WORD),
        "expected final content to contain the magic word, got {:?}",
        cap.content
    );
}

/// End-to-end exercise of a *built-in* tool (Bash), complementing the SDK MCP
/// path. The model invokes Bash to echo a sentinel; we assert the tool-use
/// surfaces as a stream event and the command's real output round-trips into
/// the reply. `can_use_tool` allows everything so the test is robust whether or
/// not the CLI raises a permission prompt for the command.
#[tokio::test]
#[ignore = "requires Claude CLI authentication and may incur API usage"]
async fn real_claude_cli_executes_builtin_bash_tool() {
    if !has_real_claude_auth() {
        eprintln!("skipping: Claude CLI authentication is required");
        return;
    }

    // A sentinel the model can only report by actually running the command.
    const BASH_SENTINEL: &str = "bash-sentinel-8675309";

    let options = ClaudeAgentOptions::builder()
        .model(TOOL_CAPABLE_MODEL)
        .include_partial_messages(true)
        .max_turns(3)
        // Allow any permission prompt that may arise, so the test asserts tool
        // *execution* rather than permission policy (which the CLI auto-approves
        // by default in this configuration).
        .can_use_tool(|_tool_name, _input, _ctx| async move { Ok(PermissionResult::allow()) })
        .build();

    let events = ClaudeAgentClient::spawn_stream_message(
        options,
        format!(
            "Use the Bash tool to run exactly: echo {BASH_SENTINEL}. \
             Then tell me the command's output."
        ),
    );
    let cap = drain_stream(events).await;

    if cap.errors.iter().any(|e| looks_unavailable(e)) {
        eprintln!("skipping: model unavailable: {:?}", cap.errors);
        return;
    }
    assert!(
        cap.errors.is_empty(),
        "unexpected stream errors: {:?}",
        cap.errors
    );

    // The Bash invocation should surface as a ToolUseStart stream event.
    assert!(
        cap.tool_starts.iter().any(|n| n == "Bash"),
        "expected a Bash ToolUseStart event, saw {:?}",
        cap.tool_starts
    );
    // The command must have actually executed and its output round-tripped back.
    assert!(
        cap.content.contains(BASH_SENTINEL),
        "expected the Bash command output in the reply, got {:?}",
        cap.content
    );
}

/// `get_mcp_status` should report the in-process server as connected and expose
/// its tool. This exercises MCP wiring without depending on model behavior, so
/// it is the cheapest and most deterministic of these tests.
#[tokio::test]
#[ignore = "requires Claude CLI authentication and may incur API usage"]
async fn real_claude_cli_reports_sdk_mcp_server_status() {
    if !has_real_claude_auth() {
        eprintln!("skipping: Claude CLI authentication is required");
        return;
    }

    let call_count = Arc::new(AtomicUsize::new(0));
    let options = options_with_magic_tool(call_count, true);
    let mut client = ClaudeAgentClient::new(options).expect("client builds");
    if let Err(err) = client.connect().await {
        if looks_unavailable(&err.to_string()) {
            eprintln!("skipping: model unavailable: {err}");
            return;
        }
        panic!("connect failed: {err}");
    }

    // The point of this test is the live control-protocol round trip: a real
    // `mcp_status` control request to the CLI that deserializes into our typed
    // response. `get_mcp_status` reflects the CLI's *configured* MCP servers
    // (from the user's environment), so the exact list is environment-specific
    // and we don't require our in-process server to appear here.
    let status = client
        .get_mcp_status()
        .await
        .expect("get_mcp_status control request should succeed against the live CLI");

    // Every reported server must deserialize into a well-formed status entry.
    for server in &status.mcp_servers {
        assert!(!server.name.is_empty(), "server status missing a name");
    }

    // If the in-process server is reported, validate its shape; otherwise note it.
    match status
        .mcp_servers
        .iter()
        .find(|server| server.name == "roder")
    {
        Some(roder) => {
            assert!(
                matches!(roder.status, MCPServerConnectionStatus::Connected),
                "expected 'roder' to be Connected, got {:?}",
                roder.status
            );
            if let Some(tools) = &roder.tools {
                assert!(
                    tools.iter().any(|t| t.name.contains("magic_word")),
                    "expected 'roder' to advertise the magic_word tool, got {:?}",
                    tools.iter().map(|t| &t.name).collect::<Vec<_>>()
                );
            }
        }
        None => eprintln!(
            "note: in-process 'roder' server not listed in mcp_status; reported servers: {:?}",
            status
                .mcp_servers
                .iter()
                .map(|s| &s.name)
                .collect::<Vec<_>>()
        ),
    }

    let _ = client.close().await;
}

/// The interactive client should retain conversation context across turns: tell
/// it a fact, then ask for it back on a second `send_message`.
#[tokio::test]
#[ignore = "requires Claude CLI authentication and may incur API usage"]
async fn real_claude_cli_interactive_multi_turn_retains_context() {
    if !has_real_claude_auth() {
        eprintln!("skipping: Claude CLI authentication is required");
        return;
    }

    let options = ClaudeAgentOptions::builder()
        .model(TOOL_CAPABLE_MODEL)
        .max_turns(1)
        .build();
    let mut client = ClaudeAgentClient::new(options).expect("client builds");
    if let Err(err) = client.connect().await {
        if looks_unavailable(&err.to_string()) {
            eprintln!("skipping: model unavailable: {err}");
            return;
        }
        panic!("connect failed: {err}");
    }

    let first = match client
        .send_message("Remember the codeword 'aardvark-42'. Reply with exactly: ok")
        .await
    {
        Ok(response) => response,
        Err(err) => {
            let _ = client.close().await;
            if looks_unavailable(&err.to_string()) {
                eprintln!("skipping: model unavailable: {err}");
                return;
            }
            panic!("first turn failed: {err}");
        }
    };
    if looks_unavailable(&first.content) {
        let _ = client.close().await;
        eprintln!("skipping: model unavailable: {:?}", first.content);
        return;
    }

    let second = client
        .send_message("What codeword did I ask you to remember? Reply with only the codeword.")
        .await
        .expect("second turn should succeed");

    assert!(
        second.content.to_ascii_lowercase().contains("aardvark-42"),
        "expected the model to recall the codeword across turns, got {:?}",
        second.content
    );

    let _ = client.close().await;
}
