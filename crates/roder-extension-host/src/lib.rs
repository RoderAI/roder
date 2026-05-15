use std::path::PathBuf;
use std::sync::Arc;

use futures::stream;
use roder_api::catalog::{PROVIDER_CODEX, PROVIDER_MOCK, models_for_codex, models_for_provider};
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistry, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::inference::*;
use roder_ext_anthropic::AnthropicExtension;
use roder_ext_gemini::GeminiExtension;
use roder_ext_jsonl_session::JsonlSessionExtension;
use roder_ext_memory::MemoryExtension;
use roder_ext_openai_responses::{OpenAiResponsesEngine, OpenAiResponsesExtension};
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
    builder.install(CodexOAuthProviderExtension)?;

    if let Some(openai_key) = config.openai_api_key {
        builder.install(OpenAiResponsesExtension::new(openai_key))?;
    }
    if let Some(anthropic_key) = config.anthropic_api_key {
        builder.install(AnthropicExtension::new(anthropic_key))?;
    }
    if let Some(gemini_key) = config.gemini_api_key {
        builder.install(GeminiExtension::new(gemini_key))?;
    }

    builder.tool_contributor(roder_tools::echo_tool_contributor());

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
            id: "roder-ext-mock-provider".to_string(),
            name: "Mock Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Deterministic local provider for tests and offline development".to_string(),
            ),
            provides: vec![ProvidedService::InferenceEngine(PROVIDER_MOCK.to_string())],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(FakeInferenceEngine));
        Ok(())
    }
}

struct CodexOAuthProviderExtension;

impl RoderExtension for CodexOAuthProviderExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-codex-oauth-provider".to_string(),
            name: "Codex OAuth Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Codex provider backed by ChatGPT OAuth".to_string()),
            provides: vec![ProvidedService::InferenceEngine(PROVIDER_CODEX.to_string())],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(CodexOAuthInferenceEngine));
        Ok(())
    }
}

struct CodexOAuthInferenceEngine;

#[async_trait::async_trait]
impl InferenceEngine for CodexOAuthInferenceEngine {
    fn id(&self) -> roder_api::extension::InferenceEngineId {
        PROVIDER_CODEX.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: false,
            tool_calls: false,
            parallel_tool_calls: false,
            reasoning_summaries: true,
            structured_output: true,
            image_input: false,
            prompt_cache: true,
            provider_metadata: true,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Codex".to_string(),
            description: Some("ChatGPT account provider for Codex models".to_string()),
            auth_type: ProviderAuthType::OAuth,
            auth_label: Some("ChatGPT Plus/Pro".to_string()),
            recommended: true,
            sort_order: 10,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_codex(false))
    }

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let Some((access_token, account_id)) = roder_codex_auth::access_token().await? else {
            anyhow::bail!("codex auth is missing; run `roder auth login codex`")
        };
        let mut headers = vec![
            ("originator".to_string(), "codex_cli_rs".to_string()),
            (
                "User-Agent".to_string(),
                "codex_cli_rs/0.1.0 roder".to_string(),
            ),
        ];
        if let Some(account_id) = account_id {
            headers.push(("ChatGPT-Account-Id".to_string(), account_id));
        }
        OpenAiResponsesEngine::new_with_config(
            access_token,
            PROVIDER_CODEX,
            "https://chatgpt.com/backend-api/codex",
            headers,
        )
        .stream_turn(ctx, request)
        .await
    }
}

struct FakeInferenceEngine;

#[async_trait::async_trait]
impl InferenceEngine for FakeInferenceEngine {
    fn id(&self) -> roder_api::extension::InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Mock".to_string(),
            description: Some("Local deterministic provider for tests".to_string()),
            auth_type: ProviderAuthType::None,
            auth_label: None,
            recommended: false,
            sort_order: 1_000,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_MOCK, true))
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

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::catalog::{PROVIDER_ANTHROPIC, PROVIDER_GEMINI, PROVIDER_OPENAI};

    #[test]
    fn default_registry_without_keys_has_mock_provider() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
        assert!(registry.inference_engine(PROVIDER_MOCK).is_some());
    }

    #[test]
    fn default_registry_with_keys_has_gode_provider_ids() {
        let registry = build_default_registry(DefaultRegistryConfig {
            openai_api_key: Some("openai".to_string()),
            anthropic_api_key: Some("anthropic".to_string()),
            gemini_api_key: Some("gemini".to_string()),
            session_dir: None,
        })
        .unwrap();
        for provider in [
            PROVIDER_MOCK,
            PROVIDER_OPENAI,
            PROVIDER_CODEX,
            PROVIDER_ANTHROPIC,
            PROVIDER_GEMINI,
        ] {
            assert!(
                registry.inference_engine(provider).is_some(),
                "missing {provider}"
            );
        }
    }
}
