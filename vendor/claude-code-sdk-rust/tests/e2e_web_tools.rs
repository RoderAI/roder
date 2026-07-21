//! End-to-end tests that exercise the built-in web tools (`WebSearch` and
//! `WebFetch`) against a live, authenticated `claude` CLI.
//!
//! These are `#[ignore]`d like the other e2e suites: they spawn the real CLI,
//! reach the public internet, and incur API usage. Run them explicitly with:
//!
//! ```bash
//! cargo test --test e2e_web_tools -- --ignored --nocapture
//! ```
//!
//! In the Claude Code CLI, web search and web fetch are *built-in* tools named
//! `WebSearch` and `WebFetch` (they surface as `ToolUseStart` stream events,
//! distinct from the API-level `web_search`/`web_fetch` server tools typed by
//! `ServerToolName`). These tests assert the tool-call plumbing: that the model
//! actually invokes the web tool and that real fetched/searched content round-
//! trips back into the reply. Each test skips gracefully (rather than failing)
//! when the model is gated or the account lacks web-tool access, so a live run
//! is green whenever the environment simply can't reach the tool.

use std::time::Duration;

use claude_code_sdk_rust::{ClaudeAgentClient, ClaudeAgentOptions, PermissionResult, StreamEvent};

/// A widely-available model that supports tool use, matching the other suites.
const TOOL_CAPABLE_MODEL: &str = "claude-haiku-4-5-20251001";

/// Web round-trips (search/fetch) can be slow; give them a generous budget so a
/// wedged turn fails fast instead of hanging CI.
const STREAM_TIMEOUT: Duration = Duration::from_secs(180);

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

/// Detect signals that the web tool itself is disabled/unavailable for this
/// account or sandbox, so a web test can skip rather than fail when the tool
/// simply isn't reachable (e.g. web access turned off, offline sandbox).
fn looks_web_unavailable(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("web search is not")
        || lowered.contains("websearch is not")
        || lowered.contains("web fetch is not")
        || lowered.contains("webfetch is not")
        || lowered.contains("not enabled")
        || lowered.contains("not available")
        || lowered.contains("don't have access to the web")
        || lowered.contains("do not have access to the web")
        || lowered.contains("don't have the ability to")
        || lowered.contains("unable to access the internet")
        || lowered.contains("cannot access the internet")
        || lowered.contains("no internet")
}

/// Drain a stream to completion (bounded), collecting the signals we assert on.
struct Captured {
    content: String,
    tool_starts: Vec<String>,
    errors: Vec<String>,
    saw_complete: bool,
}

impl Captured {
    /// Case-insensitive check for a tool-use whose name contains `needle`.
    fn used_tool(&self, needle: &str) -> bool {
        let needle = needle.to_ascii_lowercase();
        self.tool_starts
            .iter()
            .any(|name| name.to_ascii_lowercase().contains(&needle))
    }
}

async fn drain_stream(mut events: tokio::sync::mpsc::UnboundedReceiver<StreamEvent>) -> Captured {
    let mut cap = Captured {
        content: String::new(),
        tool_starts: Vec::new(),
        errors: Vec::new(),
        saw_complete: false,
    };
    let _ = tokio::time::timeout(STREAM_TIMEOUT, async {
        while let Some(event) = events.recv().await {
            match event {
                StreamEvent::ContentChunk(text) => cap.content.push_str(&text),
                StreamEvent::ToolUseStart { name, .. } => cap.tool_starts.push(name),
                StreamEvent::Complete(_) | StreamEvent::TurnComplete(_) => cap.saw_complete = true,
                StreamEvent::Error(message) => cap.errors.push(message),
                _ => {}
            }
        }
    })
    .await;
    cap
}

/// Options that enable and pre-authorize the built-in web tools. Built-ins stay
/// enabled (we do NOT call `.tools(Vec::new())`); `WebSearch`/`WebFetch` are
/// pre-approved via `allowed_tools` and `can_use_tool` allows everything as a
/// backstop so the test asserts tool *execution* rather than permission policy.
fn web_tool_options() -> ClaudeAgentOptions {
    ClaudeAgentOptions::builder()
        .model(TOOL_CAPABLE_MODEL)
        .include_partial_messages(true)
        .max_turns(4)
        .allowed_tools(vec!["WebSearch".to_string(), "WebFetch".to_string()])
        .can_use_tool(|_tool_name, _input, _ctx| async move { Ok(PermissionResult::allow()) })
        .build()
}

/// The model should invoke the built-in `WebSearch` tool and incorporate a real
/// search result into its reply. Uses a stable, well-known fact (the official
/// Rust site domain) so the round-tripped content is checkable.
#[tokio::test]
#[ignore = "requires Claude CLI authentication, web access, and may incur API usage"]
async fn real_claude_cli_uses_web_search_tool() {
    if !has_real_claude_auth() {
        eprintln!("skipping: Claude CLI authentication is required");
        return;
    }

    let events = ClaudeAgentClient::spawn_stream_message(
        web_tool_options(),
        "Use the WebSearch tool to find the official Rust programming language \
         website, then reply with the website's domain (for example example.com) \
         and nothing else.",
    );
    let cap = drain_stream(events).await;

    if cap.errors.iter().any(|e| looks_unavailable(e)) {
        eprintln!("skipping: model unavailable: {:?}", cap.errors);
        return;
    }
    if cap.errors.iter().any(|e| looks_web_unavailable(e)) {
        eprintln!("skipping: web tool unavailable: {:?}", cap.errors);
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

    // If the model never invoked the tool and instead reported it can't reach
    // the web, treat that as an environment skip rather than a hard failure.
    if !cap.used_tool("websearch") && !cap.used_tool("web_search") {
        if looks_web_unavailable(&cap.content) {
            eprintln!("skipping: web search unavailable: {:?}", cap.content);
            return;
        }
        panic!(
            "expected a WebSearch ToolUseStart event; tools seen: {:?}, content: {:?}",
            cap.tool_starts, cap.content
        );
    }

    // The search result should round-trip: the official Rust site is rust-lang.org.
    assert!(
        cap.content.to_ascii_lowercase().contains("rust-lang.org"),
        "expected the web search result (rust-lang.org) in the reply, got {:?}",
        cap.content
    );
}

/// The model should invoke the built-in `WebFetch` tool against a stable URL and
/// report its content. `https://example.com` reliably contains "Example Domain".
#[tokio::test]
#[ignore = "requires Claude CLI authentication, web access, and may incur API usage"]
async fn real_claude_cli_uses_web_fetch_tool() {
    if !has_real_claude_auth() {
        eprintln!("skipping: Claude CLI authentication is required");
        return;
    }

    let events = ClaudeAgentClient::spawn_stream_message(
        web_tool_options(),
        "Use the WebFetch tool to fetch https://example.com and reply with the \
         main heading text on that page and nothing else.",
    );
    let cap = drain_stream(events).await;

    if cap.errors.iter().any(|e| looks_unavailable(e)) {
        eprintln!("skipping: model unavailable: {:?}", cap.errors);
        return;
    }
    if cap.errors.iter().any(|e| looks_web_unavailable(e)) {
        eprintln!("skipping: web tool unavailable: {:?}", cap.errors);
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

    if !cap.used_tool("webfetch") && !cap.used_tool("web_fetch") {
        if looks_web_unavailable(&cap.content) {
            eprintln!("skipping: web fetch unavailable: {:?}", cap.content);
            return;
        }
        panic!(
            "expected a WebFetch ToolUseStart event; tools seen: {:?}, content: {:?}",
            cap.tool_starts, cap.content
        );
    }

    // example.com's only heading is "Example Domain".
    assert!(
        cap.content.to_ascii_lowercase().contains("example domain"),
        "expected the fetched page heading ('Example Domain') in the reply, got {:?}",
        cap.content
    );
}
