use claude_code_sdk_rust::{query, ClaudeAgentClient, ClaudeAgentOptions, StreamEvent};

const CLAUDE_HAIKU_4_5_MODEL: &str = "claude-haiku-4-5-20251001";
const CLAUDE_FABLE_5_MODEL: &str = "claude-fable-5";

fn has_real_claude_auth() -> bool {
    std::env::var_os("ANTHROPIC_API_KEY").is_some()
        || std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN").is_some()
        || has_logged_in_claude_cli()
}

// The Claude Code CLI stores interactive-login credentials outside the
// environment (macOS keychain / ~/.claude). A working `claude --version`
// plus a user-level config file is the best env-free signal that the CLI
// can authenticate. These tests are #[ignore]d, so this only runs when a
// developer opts in with `--ignored`.
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

#[tokio::test]
#[ignore = "requires Claude CLI authentication and may incur API usage"]
async fn real_claude_cli_query_smoke() {
    if !has_real_claude_auth() {
        eprintln!("skipping: Claude CLI authentication is required");
        return;
    }

    let options = ClaudeAgentOptions::builder().max_turns(1).build();
    let result = query("Reply with exactly: pong", Some(options))
        .await
        .expect("real Claude CLI query should succeed");

    assert!(
        result.content.to_ascii_lowercase().contains("pong"),
        "expected response to contain pong, got {:?}",
        result.content
    );
}

#[tokio::test]
#[ignore = "requires Claude CLI authentication and may incur API usage"]
async fn real_claude_cli_haiku_4_5_query_smoke() {
    if !has_real_claude_auth() {
        eprintln!("skipping: Claude CLI authentication is required");
        return;
    }

    let options = ClaudeAgentOptions::builder()
        .model(CLAUDE_HAIKU_4_5_MODEL)
        .max_turns(1)
        .build();
    let result = query("Reply with exactly: haiku-pong", Some(options))
        .await
        .expect("real Claude CLI Haiku 4.5 query should succeed");

    assert_eq!(
        result.content.trim(),
        "haiku-pong",
        "expected exact Haiku 4.5 response, got {:?}",
        result.content
    );
}

#[tokio::test]
#[ignore = "requires Claude CLI authentication and may incur API usage"]
async fn real_claude_cli_fable_5_query_smoke() {
    if !has_real_claude_auth() {
        eprintln!("skipping: Claude CLI authentication is required");
        return;
    }

    let options = ClaudeAgentOptions::builder()
        .model(CLAUDE_FABLE_5_MODEL)
        .max_turns(1)
        .build();
    let result = query("Reply with exactly: fable-pong", Some(options))
        .await
        .expect("real Claude CLI Fable 5 query should succeed");

    assert_eq!(
        result.content.trim(),
        "fable-pong",
        "expected exact Fable 5 response, got {:?}",
        result.content
    );
}

/// Mirrors how Roder's claude-code provider drives the SDK: streaming via
/// `spawn_stream_message` with partial messages and an effort level. Verifies
/// Fable 5 produces incremental content chunks and a final Complete event.
#[tokio::test]
#[ignore = "requires Claude CLI authentication and may incur API usage"]
async fn real_claude_cli_fable_5_streaming_smoke() {
    if !has_real_claude_auth() {
        eprintln!("skipping: Claude CLI authentication is required");
        return;
    }

    let options = ClaudeAgentOptions::builder()
        .model(CLAUDE_FABLE_5_MODEL)
        .include_partial_messages(true)
        .effort(claude_code_sdk_rust::EffortLevel::Medium)
        .max_turns(1)
        .build();
    let mut events =
        ClaudeAgentClient::spawn_stream_message(options, "Reply with exactly: fable-stream-pong");

    let mut content = String::new();
    let mut completes = Vec::new();
    let mut errors = Vec::new();
    while let Some(event) = events.recv().await {
        match event {
            StreamEvent::ContentChunk(text) => content.push_str(&text),
            StreamEvent::Complete(response) => completes.push(response),
            StreamEvent::Error(message) => errors.push(message),
            _ => {}
        }
    }

    assert!(errors.is_empty(), "stream errors: {errors:?}");
    assert!(
        !completes.is_empty(),
        "expected at least one Complete event from the Fable 5 stream"
    );
    assert!(
        content.contains("fable-stream-pong"),
        "expected streamed content to contain fable-stream-pong, got {content:?}"
    );
    // The AssistantMsg-derived Complete carries the model; ResultMsg-derived
    // Completes have an empty model. At least one must identify Fable.
    assert!(
        completes
            .iter()
            .any(|response| response.model.contains("fable")),
        "expected a Complete event with a Fable model, got {:?}",
        completes
            .iter()
            .map(|response| response.model.as_str())
            .collect::<Vec<_>>()
    );
}
