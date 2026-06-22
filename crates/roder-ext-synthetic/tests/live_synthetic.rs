use futures::StreamExt;
use roder_api::catalog::PROVIDER_SYNTHETIC;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEvent, InferenceProviderContext, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_synthetic::{SyntheticConfig, SyntheticExtension};
use serde_json::json;

#[tokio::test]
#[ignore = "requires RODER_SYNTHETIC_LIVE=1, SYNTHETIC_API_KEY, optional SYNTHETIC_LIVE_MODEL"]
async fn live_synthetic_lists_models_and_completes_short_turn() {
    if std::env::var("RODER_SYNTHETIC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live Synthetic smoke: set RODER_SYNTHETIC_LIVE=1");
        return;
    }
    let api_key = std::env::var("SYNTHETIC_API_KEY")
        .expect("SYNTHETIC_API_KEY is required for live Synthetic smoke");
    // Default to the documented recommended alias when no explicit model is set.
    let model =
        std::env::var("SYNTHETIC_LIVE_MODEL").unwrap_or_else(|_| "syn:large:text".to_string());

    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(SyntheticExtension::new(SyntheticConfig {
            api_key: Some(api_key),
            base_url: std::env::var("SYNTHETIC_BASE_URL").ok(),
        }))
        .unwrap();
    let registry = builder.build().unwrap();
    let engine = registry.inference_engine(PROVIDER_SYNTHETIC).unwrap();

    let models = engine
        .list_models(InferenceProviderContext {
            provider_id: PROVIDER_SYNTHETIC,
        })
        .await
        .unwrap();
    assert!(
        !models.is_empty(),
        "Synthetic model listing returned no models"
    );

    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: PROVIDER_SYNTHETIC.to_string(),
            model,
        },
        instructions: InstructionBundle {
            system: Some("Answer very briefly.".to_string()),
            developer: None,
            developer_context: None,
        },
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            "Reply with exactly: synthetic ok",
        ))],
        tools: Vec::new(),
        tool_choice: ToolChoice::None,
        reasoning: ReasoningConfig {
            enabled: false,
            level: None,
        },
        // Synthetic's default models (e.g. GLM-5.2) stream `reasoning_content`
        // before any answer `content`, so a tiny budget can hit
        // `finish_reason: length` mid-reasoning and yield no visible text. Give
        // enough headroom for reasoning plus a short reply.
        output: OutputConfig {
            max_tokens: Some(512),
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
                thread_id: "live-synthetic",
                turn_id: "turn-live-synthetic",
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

    assert!(!text.trim().is_empty(), "Synthetic returned no text");
    assert!(saw_usage, "Synthetic live response did not include usage");
}
