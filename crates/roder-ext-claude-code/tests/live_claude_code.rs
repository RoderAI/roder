//! Opt-in live Claude Code provider validation (roadmap phase 78). Ignored
//! by default; runs only with `RODER_CLAUDE_CODE_LIVE=1` and an installed,
//! authenticated `claude` CLI. Output is limited to a short sentinel turn —
//! no raw SDK frames, secrets, or private filesystem content is asserted or
//! printed beyond the model's reply text.

use futures::StreamExt;
use roder_api::inference::{
    AgentInferenceRequest, HostedWebSearchConfig, InferenceEngine, InferenceEvent,
    InferenceTurnContext, InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig,
    RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_claude_code::{ClaudeCodeConfig, ClaudeCodeEngine};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires RODER_CLAUDE_CODE_LIVE=1 and an authenticated local claude CLI"]
async fn live_claude_code_short_text_turn_completes() {
    if std::env::var("RODER_CLAUDE_CODE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_CLAUDE_CODE_LIVE=1 to run live Claude Code provider tests");
        return;
    }

    let engine = ClaudeCodeEngine::new(ClaudeCodeConfig::default());
    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: "claude-code".to_string(),
            model: std::env::var("RODER_CLAUDE_CODE_MODEL").unwrap_or_else(|_| "haiku".to_string()),
        },
        instructions: InstructionBundle {
            system: Some("Answer in one short sentence.".to_string()),
            developer: None,
            developer_context: None,
        },
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            "Reply with exactly: CLAUDE_CODE_LIVE_OK",
        ))],
        tools: Vec::new(),
        tool_choice: ToolChoice::None,
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
                thread_id: "live-claude-code-test",
                turn_id: "turn-1",
                tool_executor: None,
            },
            request,
        )
        .await
        .expect("start live Claude Code turn");

    let mut text = String::new();
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event.expect("live stream event") {
            InferenceEvent::MessageDelta(delta) => text.push_str(&delta.text),
            InferenceEvent::Completed(_) => {
                completed = true;
                break;
            }
            InferenceEvent::Failed(failure) => {
                panic!("live Claude Code turn failed: {}", failure.message)
            }
            _ => {}
        }
    }

    assert!(completed, "live turn must complete");
    assert!(
        !text.trim().is_empty(),
        "live turn must stream assistant text"
    );
    eprintln!("live Claude Code reply: {:?}", text.trim());
}
