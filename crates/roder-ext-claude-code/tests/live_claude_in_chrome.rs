//! Opt-in live validation that the Claude Code provider can register against
//! the local "Claude in Chrome" extension and let the model invoke its browser
//! tools. Ignored by default; runs only with `RODER_CLAUDE_CODE_CHROME_LIVE=1`
//! and an installed, authenticated `claude` CLI whose Chrome extension is
//! paired. Output is limited to the surfaced tool name -- no page content,
//! screenshots, or secrets are asserted or printed.
//!
//! This exercises the full harness path: the provider spawns `claude` with the
//! Claude-in-Chrome integration forced on (`CLAUDE_CODE_ENABLE_CFC=1`) and
//! API-key auth blanked so the CLI uses subscription auth, advertises the
//! browser tools alongside the Roder MCP tools, pre-authorizes them through
//! `can_use_tool`, and surfaces the resulting CLI-executed tool call as a hosted
//! tool call. A throwaway Roder tool is advertised so the provider disables the
//! built-in tool set (`--tools ""`) exactly as it does for a real turn, which is
//! what makes the browser tools available directly instead of behind ToolSearch.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use roder_api::inference::{
    AgentInferenceRequest, HostedWebSearchConfig, InferenceEngine, InferenceEvent,
    InferenceTurnContext, InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig,
    RuntimeHints, ToolCallCompleted, TurnToolExecutor, TurnToolOutcome,
};
use roder_api::tools::{ToolChoice, ToolSpec};
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_claude_code::{ClaudeCodeConfig, ClaudeCodeEngine};

struct NoopExecutor;

#[async_trait]
impl TurnToolExecutor for NoopExecutor {
    async fn execute(&self, _call: ToolCallCompleted) -> anyhow::Result<TurnToolOutcome> {
        Ok(TurnToolOutcome {
            result: "ok".to_string(),
            is_error: false,
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires RODER_CLAUDE_CODE_CHROME_LIVE=1, an authenticated claude CLI, and a paired Chrome extension"]
async fn live_claude_in_chrome_browser_tool_is_invoked() {
    if std::env::var("RODER_CLAUDE_CODE_CHROME_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_CLAUDE_CODE_CHROME_LIVE=1 to run the live Claude-in-Chrome test");
        return;
    }

    let engine = ClaudeCodeEngine::new(ClaudeCodeConfig {
        enable_claude_in_chrome: Some(true),
        ..ClaudeCodeConfig::default()
    });

    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: "claude-code".to_string(),
            model: std::env::var("RODER_CLAUDE_CODE_MODEL").unwrap_or_else(|_| "sonnet".to_string()),
        },
        instructions: InstructionBundle {
            system: Some(
                "You have the claude-in-chrome browser tools available. When asked about the \
                 browser, you MUST call a claude-in-chrome tool rather than guessing."
                    .to_string(),
            ),
            developer: None,
            developer_context: None,
        },
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            "Use the claude-in-chrome browser tools to read the current browser context \
             (the list of open tabs). Then reply with a one-line summary.",
        ))],
        // Advertise a throwaway Roder tool so the provider disables the built-in
        // CLI tool set the same way it does for a real turn.
        tools: vec![ToolSpec {
            name: "noop".to_string(),
            description: "Does nothing; present only to mirror a real tool turn.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }],
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig::default(),
        runtime: RuntimeHints {
            hosted_web_search: HostedWebSearchConfig::disabled(),
            ..RuntimeHints::default()
        },
        metadata: serde_json::json!({}),
    };

    let mut stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: "live-claude-in-chrome",
                turn_id: "turn-1",
                tool_executor: Some(Arc::new(NoopExecutor)),
            },
            request,
        )
        .await
        .expect("start live Claude-in-Chrome turn");

    let mut hosted_browser_tools: Vec<String> = Vec::new();
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event.expect("live stream event") {
            InferenceEvent::HostedToolCallStarted(call)
                if call.name.starts_with("mcp__claude-in-chrome__")
                    || call.name.starts_with("mcp__Claude_in_Chrome__") =>
            {
                hosted_browser_tools.push(call.name);
            }
            InferenceEvent::Completed(_) => {
                completed = true;
                break;
            }
            InferenceEvent::Failed(failure) => {
                panic!("live Claude-in-Chrome turn failed: {}", failure.message)
            }
            _ => {}
        }
    }

    assert!(completed, "live turn must complete");
    assert!(
        !hosted_browser_tools.is_empty(),
        "the model must invoke at least one claude-in-chrome browser tool"
    );
    eprintln!(
        "live Claude-in-Chrome invoked browser tools: {:?}",
        hosted_browser_tools
    );
}
