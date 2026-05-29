use futures::StreamExt;
use roder_api::catalog::{PROVIDER_XIAOMI_MIMO, PROVIDER_XIAOMI_MIMO_TOKEN_PLAN};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEvent, InferenceTurnContext, InstructionBundle, ModelSelection,
    OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::speech::{SpeechProviderContext, SpeechSynthesisRequest};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_xiaomi_mimo::{XiaomiMimoConfig, XiaomiMimoExtension};

#[tokio::test]
#[ignore = "requires RODER_XIAOMI_MIMO_LIVE=1 and a real Xiaomi MiMo API key"]
async fn ordinary_mimo_streaming_chat_completions_smoke() {
    require_flag("RODER_XIAOMI_MIMO_LIVE");
    let api_key = env("MIMO_API_KEY").or_else(|| env("RODER_XIAOMI_MIMO_API_KEY"));
    let registry = registry(XiaomiMimoConfig {
        api_key,
        ..XiaomiMimoConfig::default()
    });
    let engine = registry
        .inference_engine(PROVIDER_XIAOMI_MIMO)
        .expect("xiaomi-mimo engine");
    let mut stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: "live-xiaomi",
                turn_id: "turn-1",
                tool_executor: None,
            },
            request(PROVIDER_XIAOMI_MIMO, "mimo-v2.5-pro"),
        )
        .await
        .expect("live Xiaomi chat stream");

    let mut saw_delta = false;
    while let Some(event) = stream.next().await {
        if matches!(
            event.expect("stream event"),
            InferenceEvent::MessageDelta(_)
        ) {
            saw_delta = true;
        }
    }
    assert!(saw_delta, "expected at least one message delta");
}

#[tokio::test]
#[ignore = "requires RODER_XIAOMI_MIMO_TOKEN_PLAN_LIVE=1, a tp- key, and a Token Plan base URL"]
async fn token_plan_streaming_chat_completions_smoke() {
    require_flag("RODER_XIAOMI_MIMO_TOKEN_PLAN_LIVE");
    let registry = registry(XiaomiMimoConfig {
        token_plan_api_key: env("MIMO_TOKEN_PLAN_API_KEY")
            .or_else(|| env("RODER_XIAOMI_MIMO_TOKEN_PLAN_API_KEY")),
        token_plan_base_url: env("RODER_XIAOMI_MIMO_TOKEN_PLAN_BASE_URL")
            .or_else(|| env("MIMO_TOKEN_PLAN_BASE_URL")),
        ..XiaomiMimoConfig::default()
    });
    let engine = registry
        .inference_engine(PROVIDER_XIAOMI_MIMO_TOKEN_PLAN)
        .expect("xiaomi-mimo-token-plan engine");
    let mut stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: "live-xiaomi-token-plan",
                turn_id: "turn-1",
                tool_executor: None,
            },
            request(PROVIDER_XIAOMI_MIMO_TOKEN_PLAN, "mimo-v2.5-pro"),
        )
        .await
        .expect("live Xiaomi Token Plan chat stream");

    while let Some(event) = stream.next().await {
        event.expect("stream event");
    }
}

#[tokio::test]
#[ignore = "requires RODER_XIAOMI_MIMO_TTS_LIVE=1 and a real Xiaomi MiMo API key"]
async fn ordinary_mimo_tts_smoke() {
    require_flag("RODER_XIAOMI_MIMO_TTS_LIVE");
    let registry = registry(XiaomiMimoConfig {
        api_key: env("MIMO_API_KEY").or_else(|| env("RODER_XIAOMI_MIMO_API_KEY")),
        ..XiaomiMimoConfig::default()
    });
    let synthesizer = registry
        .speech_synthesizer(PROVIDER_XIAOMI_MIMO)
        .expect("xiaomi-mimo speech synthesizer");
    let result = synthesizer
        .synthesize(
            SpeechProviderContext {
                provider_id: PROVIDER_XIAOMI_MIMO,
            },
            SpeechSynthesisRequest {
                model: "mimo-v2.5-tts".to_string(),
                text: "Hello from Roder.".to_string(),
                voice: Some("Chloe".to_string()),
                audio_format: Some("wav".to_string()),
                prompt: None,
                voice_sample: None,
                metadata: Default::default(),
            },
        )
        .await
        .expect("live Xiaomi TTS");

    assert!(!result.audio.bytes.is_empty());
}

fn registry(config: XiaomiMimoConfig) -> roder_api::extension::ExtensionRegistry {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.install(XiaomiMimoExtension::new(config)).unwrap();
    builder.build().unwrap()
}

fn request(provider: &str, model: &str) -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: provider.to_string(),
            model: model.to_string(),
        },
        instructions: InstructionBundle::default(),
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            "Reply with exactly: pong",
        ))],
        tools: Vec::new(),
        tool_choice: ToolChoice::None,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig {
            max_tokens: Some(16),
            temperature: Some(0.0),
            ..OutputConfig::default()
        },
        runtime: RuntimeHints::default(),
        metadata: serde_json::json!({}),
    }
}

fn require_flag(key: &str) {
    assert_eq!(std::env::var(key).ok().as_deref(), Some("1"));
}

fn env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
