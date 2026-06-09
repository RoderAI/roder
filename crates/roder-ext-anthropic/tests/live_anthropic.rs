use futures::StreamExt;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_anthropic::AnthropicEngine;

#[tokio::test]
#[ignore = "requires RODER_ANTHROPIC_LIVE=1 and ANTHROPIC_API_KEY"]
async fn live_anthropic_streaming_emits_incremental_message_deltas() {
    if std::env::var("RODER_ANTHROPIC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_ANTHROPIC_LIVE=1 to run live Anthropic streaming tests");
        return;
    }
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY must be set");

    let engine = AnthropicEngine::new(api_key);
    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: "anthropic".to_string(),
            model: "claude-haiku-4-5-20251001".to_string(),
        },
        instructions: InstructionBundle::default(),
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            "Count from 1 to 20 as words, one per line.",
        ))],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig {
            max_tokens: Some(300),
            ..OutputConfig::default()
        },
        runtime: RuntimeHints::default(),
        metadata: serde_json::json!({}),
    };

    let mut stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: "live-test",
                turn_id: "live-turn",
                tool_executor: None,
            },
            request,
        )
        .await
        .unwrap();

    let mut message_deltas = 0;
    let mut saw_usage = false;
    let mut stop_reason = None;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            InferenceEvent::MessageDelta(_) => message_deltas += 1,
            InferenceEvent::Usage(usage) => {
                saw_usage = true;
                assert!(usage.prompt_tokens > 0);
                assert!(usage.completion_tokens > 0);
            }
            InferenceEvent::Completed(metadata) => stop_reason = metadata.stop_reason,
            _ => {}
        }
    }

    // The API coalesces deltas; more than one proves the reply streamed
    // incrementally instead of arriving as a single blob.
    assert!(
        message_deltas >= 2,
        "expected incremental text deltas, got {message_deltas}"
    );
    assert!(saw_usage);
    assert_eq!(stop_reason.as_deref(), Some("end_turn"));
}

#[tokio::test]
#[ignore = "requires RODER_ANTHROPIC_LIVE=1 and ANTHROPIC_API_KEY"]
async fn live_anthropic_tools_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_ANTHROPIC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_ANTHROPIC_LIVE=1 to run live Anthropic tool smoke tests");
        return;
    }

    let has_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some();

    assert!(
        has_key,
        "live Anthropic tool smoke tests require ANTHROPIC_API_KEY"
    );
}

#[tokio::test]
#[ignore = "requires RODER_ANTHROPIC_TOOL_SEARCH_LIVE=1 and ANTHROPIC_API_KEY"]
async fn live_anthropic_tool_search_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_ANTHROPIC_TOOL_SEARCH_LIVE")
        .ok()
        .as_deref()
        != Some("1")
    {
        eprintln!("set RODER_ANTHROPIC_TOOL_SEARCH_LIVE=1 to run live Anthropic tool-search tests");
        return;
    }

    let has_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some();

    assert!(
        has_key,
        "live Anthropic tool-search smoke tests require ANTHROPIC_API_KEY"
    );
}
