use std::path::PathBuf;
use std::sync::Arc;

use futures::stream;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistry, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::inference::*;
use roder_ext_anthropic::AnthropicExtension;
use roder_ext_gemini::GeminiExtension;
use roder_ext_jsonl_session::JsonlSessionExtension;
use roder_ext_memory::MemoryExtension;
use roder_ext_openai_chat_completions::OpenAiChatCompletionsExtension;
use roder_ext_openai_responses::OpenAiResponsesExtension;
use semver::Version;

#[derive(Debug, Clone, Default)]
pub struct DefaultRegistryConfig {
    pub openai_api_key: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub gemini_api_key: Option<String>,
    pub session_dir: Option<PathBuf>,
}

pub fn build_default_registry(config: DefaultRegistryConfig) -> anyhow::Result<ExtensionRegistry> {
    let mut builder = ExtensionRegistryBuilder::new();

    builder.install(FakeProviderExtension)?;

    if let Some(openai_key) = config.openai_api_key {
        builder.install(OpenAiChatCompletionsExtension::new(openai_key.clone()))?;
        builder.install(OpenAiResponsesExtension::new(openai_key))?;
    }
    if let Some(anthropic_key) = config.anthropic_api_key {
        builder.install(AnthropicExtension::new(anthropic_key))?;
    }
    if let Some(gemini_key) = config.gemini_api_key {
        builder.install(GeminiExtension::new(gemini_key))?;
    }

    let session_dir = config
        .session_dir
        .unwrap_or_else(|| PathBuf::from(".roder").join("sessions"));
    builder.install(JsonlSessionExtension::new(session_dir))?;
    builder.install(MemoryExtension::new(PathBuf::from(".roder").join("memory")))?;

    builder.build()
}

struct FakeProviderExtension;

impl RoderExtension for FakeProviderExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-fake-provider".to_string(),
            name: "Fake Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Deterministic local provider for tests and no-key runs".to_string()),
            provides: vec![ProvidedService::InferenceEngine(
                "fake-provider".to_string(),
            )],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(FakeInferenceEngine));
        Ok(())
    }
}

struct FakeInferenceEngine;

#[async_trait::async_trait]
impl InferenceEngine for FakeInferenceEngine {
    fn id(&self) -> roder_api::extension::InferenceEngineId {
        "fake-provider".to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(vec![ModelDescriptor {
            id: "fake-model".to_string(),
            name: "Fake Model".to_string(),
            context_window: Some(128_000),
        }])
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        Ok(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "hello".to_string(),
            })),
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: " from".to_string(),
            })),
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: " roder".to_string(),
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}
