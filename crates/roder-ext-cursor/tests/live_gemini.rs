use futures::StreamExt;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_cursor::{CursorConfig, CursorInferenceEngine};
use serde_json::json;

#[tokio::test]
#[ignore = "requires RODER_CURSOR_LIVE=1 and Cursor credentials"]
async fn live_cursor_current_gemini_models() {
    assert_eq!(std::env::var("RODER_CURSOR_LIVE").as_deref(), Ok("1"));
    let engine = CursorInferenceEngine::new(CursorConfig::default());
    for model in ["gemini-3.1-pro", "gemini-4-argon"] {
        let thread = format!("cursor-gemini-{}", uuid::Uuid::new_v4());
        let proof = "CURSOR_GEMINI_ROSTER_OK";
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: "cursor".into(),
                model: model.into(),
            },
            instructions: InstructionBundle::default(),
            transcript: vec![TranscriptItem::UserMessage(UserMessage::text(format!(
                "Reply with exactly {proof} and no other text. Do not use tools."
            )))],
            tools: vec![],
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig {
                enabled: true,
                level: Some("high".into()),
            },
            output: OutputConfig::default(),
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        };
        let mut stream = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: &thread,
                    turn_id: "gemini-roster",
                    tool_executor: None,
                },
                request,
            )
            .await
            .unwrap();
        let mut text = String::new();
        let mut completed = false;
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                InferenceEvent::MessageDelta(delta) => text.push_str(&delta.text),
                InferenceEvent::Completed(_) => completed = true,
                _ => {}
            }
        }
        assert!(completed, "{model} did not complete");
        assert_eq!(text.trim(), proof, "{model}");
        eprintln!("{model}: live proof passed");
    }
}
