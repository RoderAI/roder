use futures::StreamExt;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_vertex::{VertexConfig, VertexEngine};

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[tokio::test]
#[ignore = "requires RODER_LIVE_VERTEX=1 and service-account credentials"]
async fn live_vertex_streaming_emits_incremental_message_deltas() {
    if std::env::var("RODER_LIVE_VERTEX").ok().as_deref() != Some("1") {
        eprintln!("set RODER_LIVE_VERTEX=1 to run live Vertex AI streaming tests");
        return;
    }
    let config = VertexConfig {
        credentials_path: env_non_empty("GOOGLE_APPLICATION_CREDENTIALS"),
        credentials_json: env_non_empty("VERTEX_CREDENTIALS_JSON"),
        project: env_non_empty("VERTEX_PROJECT"),
        location: env_non_empty("VERTEX_LOCATION"),
    };
    assert!(
        config.credentials_path.is_some() || config.credentials_json.is_some(),
        "live Vertex tests require GOOGLE_APPLICATION_CREDENTIALS or VERTEX_CREDENTIALS_JSON"
    );
    let model =
        env_non_empty("RODER_VERTEX_LIVE_MODEL").unwrap_or_else(|| "gemini-3.5-flash".to_string());

    let engine = VertexEngine::new(config);
    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: "vertex".to_string(),
            model,
        },
        instructions: InstructionBundle::default(),
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            "Write five numbered one-sentence facts about the ocean.",
        ))],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig {
            max_tokens: Some(2000),
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
    let mut deltas_before_completed = 0;
    let mut saw_usage = false;
    let mut completed = false;
    let mut stop_reason = None;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            InferenceEvent::MessageDelta(_) => {
                message_deltas += 1;
                if !completed {
                    deltas_before_completed += 1;
                }
            }
            InferenceEvent::Usage(usage) => {
                saw_usage = true;
                assert!(usage.prompt_tokens > 0);
                assert!(usage.completion_tokens > 0);
            }
            InferenceEvent::Completed(metadata) => {
                completed = true;
                stop_reason = metadata.stop_reason;
            }
            _ => {}
        }
    }
    eprintln!(
        "live vertex streaming: {message_deltas} MessageDelta events ({deltas_before_completed} before Completed)"
    );

    // The API coalesces deltas; more than one proves the reply streamed
    // incrementally instead of arriving as a single blob.
    assert!(
        deltas_before_completed > 1,
        "expected >1 incremental text deltas before Completed, got {deltas_before_completed}"
    );
    assert!(completed, "stream ended without a Completed event");
    assert!(saw_usage);
    assert_eq!(stop_reason.as_deref(), Some("stop"));
}
