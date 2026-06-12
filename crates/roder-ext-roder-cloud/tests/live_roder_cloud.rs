//! Opt-in live checks against a real roder.cloud deployment (or a local
//! godex `bin/dev` stack). Never run by default.
//!
//! ```sh
//! RODER_RODER_CLOUD_LIVE=1 \
//! RODER_CLOUD_API_KEY=roder_... \
//! RODER_CLOUD_BASE_URL=http://127.0.0.1:8080/v1 \
//! RODER_CLOUD_WEB_URL=http://localhost:3000 \
//! cargo test -p roder-ext-roder-cloud --test live_roder_cloud -- --ignored
//! ```

use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceProviderContext,
    InferenceTurnContext, InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig,
    RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_roder_cloud::RoderCloudEngine;

fn live_enabled() -> bool {
    std::env::var("RODER_RODER_CLOUD_LIVE").as_deref() == Ok("1")
}

fn live_engine() -> RoderCloudEngine {
    RoderCloudEngine::new(
        std::env::var("RODER_CLOUD_API_KEY").ok(),
        std::env::var("RODER_CLOUD_BASE_URL").ok(),
        std::env::var("RODER_CLOUD_WEB_URL").ok(),
    )
}

fn live_model() -> String {
    std::env::var("RODER_CLOUD_LIVE_MODEL").unwrap_or_else(|_| "roder.cloud/free".to_string())
}

#[tokio::test]
#[ignore = "requires RODER_RODER_CLOUD_LIVE=1, RODER_CLOUD_API_KEY, and RODER_CLOUD_BASE_URL"]
async fn live_models_and_free_turn() {
    if !live_enabled() {
        eprintln!("skipping: RODER_RODER_CLOUD_LIVE != 1");
        return;
    }
    let engine = live_engine();

    let models = engine
        .list_models(InferenceProviderContext {
            provider_id: "roder-cloud",
        })
        .await
        .expect("live model list");
    assert!(!models.is_empty(), "live model list must not be empty");

    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: "roder-cloud".to_string(),
            model: live_model(),
        },
        instructions: InstructionBundle {
            system: Some("Reply with one short sentence.".to_string()),
            developer: None,
            developer_context: None,
        },
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            "Say hello from roder.cloud.",
        ))],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig {
            max_tokens: Some(64),
            temperature: None,
            top_p: None,
            response_format: None,
        },
        runtime: RuntimeHints::default(),
        metadata: serde_json::Value::Null,
    };
    let stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: "live-thread",
                turn_id: "live-turn",
                tool_executor: None,
            },
            request,
        )
        .await
        .expect("live turn");

    use futures::StreamExt;
    let events = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<anyhow::Result<Vec<_>>>()
        .expect("live events");
    let text = events
        .iter()
        .filter_map(|event| match event {
            InferenceEvent::MessageDelta(delta) => Some(delta.text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(!text.is_empty(), "live turn must produce output text");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, InferenceEvent::Usage(usage) if usage.total_tokens > 0)),
        "live turn must report usage"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, InferenceEvent::Completed(_))),
        "live turn must complete"
    );
}
