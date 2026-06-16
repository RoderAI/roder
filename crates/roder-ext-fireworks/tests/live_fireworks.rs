use futures::StreamExt;
use roder_api::catalog::PROVIDER_FIREWORKS;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEvent, InferenceProviderContext, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_fireworks::{FireworksConfig, FireworksExtension};
use serde_json::json;

#[tokio::test]
#[ignore = "requires RODER_FIREWORKS_LIVE=1, FIREWORKS_API_KEY, and FIREWORKS_LIVE_MODEL"]
async fn live_fireworks_lists_models_and_completes_short_turn() {
    if std::env::var("RODER_FIREWORKS_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live Fireworks smoke: set RODER_FIREWORKS_LIVE=1");
        return;
    }
    let api_key = std::env::var("FIREWORKS_API_KEY")
        .expect("FIREWORKS_API_KEY is required for live Fireworks smoke");
    let model = std::env::var("FIREWORKS_LIVE_MODEL")
        .expect("FIREWORKS_LIVE_MODEL is required for live Fireworks smoke");

    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(FireworksExtension::new(FireworksConfig {
            api_key: Some(api_key),
            base_url: None,
        }))
        .unwrap();
    let registry = builder.build().unwrap();
    let engine = registry.inference_engine(PROVIDER_FIREWORKS).unwrap();

    let models = engine
        .list_models(InferenceProviderContext {
            provider_id: PROVIDER_FIREWORKS,
        })
        .await
        .unwrap();
    assert!(
        !models.is_empty(),
        "Fireworks model discovery returned no models"
    );

    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: PROVIDER_FIREWORKS.to_string(),
            model,
        },
        instructions: InstructionBundle {
            system: Some("Answer very briefly.".to_string()),
            developer: None,
            developer_context: None,
        },
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            "Reply with exactly: fireworks ok",
        ))],
        tools: Vec::new(),
        tool_choice: roder_api::tools::ToolChoice::None,
        reasoning: ReasoningConfig {
            enabled: false,
            level: None,
        },
        output: OutputConfig {
            max_tokens: Some(32),
            temperature: Some(0.0),
            top_p: None,
            response_format: None,
        },
        runtime: RuntimeHints::default(),
        metadata: json!({}),
    };

    let mut stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: "live-fireworks",
                turn_id: "turn-live-fireworks",
                tool_executor: None,
            },
            request,
        )
        .await
        .unwrap();

    let mut text = String::new();
    let mut saw_usage = false;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            InferenceEvent::MessageDelta(delta) => text.push_str(&delta.text),
            InferenceEvent::Usage(_) => saw_usage = true,
            _ => {}
        }
    }

    assert!(!text.trim().is_empty(), "Fireworks returned no text");
    assert!(saw_usage, "Fireworks live response did not include usage");
}
