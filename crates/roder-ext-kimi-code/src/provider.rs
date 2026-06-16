use roder_api::catalog::{PROVIDER_KIMI_CODE, models_for_provider};
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext,
    ProviderAuthType,
};
use roder_ext_openai_chat_completions::{ChatCompletionsRequestConfig, stream_chat_completions};

use crate::auth::{access_token, has_stored_tokens};

#[derive(Debug, Clone, Default)]
pub struct KimiCodeConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KimiCodeProviderSpec {
    pub provider_id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub default_base_url: &'static str,
    pub sort_order: i32,
    pub api_key_env: &'static str,
    pub api_key_aliases: &'static [&'static str],
    pub base_url_env: &'static str,
    pub base_url_aliases: &'static [&'static str],
}

impl Default for KimiCodeProviderSpec {
    fn default() -> Self {
        Self {
            provider_id: PROVIDER_KIMI_CODE,
            name: "Kimi Code",
            description: "Kimi Code (Moonshot AI) subscription inference (direct Moonshot route).",
            default_base_url: "https://api.moonshot.ai/v1",
            sort_order: 25,
            api_key_env: "KIMI_CODE_API_KEY",
            api_key_aliases: &["RODER_KIMI_CODE_API_KEY"],
            base_url_env: "RODER_KIMI_CODE_BASE_URL",
            base_url_aliases: &["KIMI_CODE_BASE_URL"],
        }
    }
}

pub struct KimiCodeInferenceEngine {
    config: KimiCodeConfig,
    spec: KimiCodeProviderSpec,
}

impl KimiCodeInferenceEngine {
    pub fn new(config: KimiCodeConfig, spec: KimiCodeProviderSpec) -> Self {
        Self { config, spec }
    }

    fn base_url(&self) -> String {
        self.config
            .base_url
            .clone()
            .or_else(|| std::env::var(self.spec.base_url_env).ok())
            .or_else(|| {
                for alias in self.spec.base_url_aliases {
                    if let Ok(v) = std::env::var(alias) {
                        return Some(v);
                    }
                }
                None
            })
            .unwrap_or_else(|| self.spec.default_base_url.to_string())
    }

    fn api_key(&self) -> Option<String> {
        self.config
            .api_key
            .clone()
            .or_else(|| std::env::var(self.spec.api_key_env).ok())
            .or_else(|| {
                for alias in self.spec.api_key_aliases {
                    if let Ok(v) = std::env::var(alias) {
                        return Some(v);
                    }
                }
                None
            })
    }

    async fn resolve_bearer(&self) -> anyhow::Result<String> {
        if let Some(api_key) = self.api_key() {
            return Ok(api_key);
        }
        if let Some(access_token) = access_token().await? {
            return Ok(access_token);
        }
        anyhow::bail!(
            "{} auth is missing; run `roder auth login kimi-code` or set {} / {}",
            self.spec.name,
            self.spec.api_key_env,
            self.spec
                .api_key_aliases
                .first()
                .copied()
                .unwrap_or("RODER_KIMI_CODE_API_KEY")
        )
    }
}

#[async_trait::async_trait]
impl InferenceEngine for KimiCodeInferenceEngine {
    fn id(&self) -> roder_api::extension::InferenceEngineId {
        self.spec.provider_id.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: false,
            structured_output: true,
            image_input: false,
            prompt_cache: false,
            provider_metadata: true,
            tool_search: false,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: self.spec.name.to_string(),
            description: Some(self.spec.description.to_string()),
            auth_type: ProviderAuthType::OAuth,
            auth_label: Some("Kimi Code subscription or API key".to_string()),
            auth_configured: Some(self.api_key().is_some() || has_stored_tokens()),
            recommended: true,
            sort_order: self.spec.sort_order,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
        Ok(models_for_provider(self.spec.provider_id, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let bearer = self.resolve_bearer().await?;
        let base_url = self.base_url();
        stream_chat_completions(
            ChatCompletionsRequestConfig::bearer(self.spec.name.to_string(), base_url, bearer),
            request,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_reports_oauth_with_api_key_or_stored_tokens() {
        let engine = KimiCodeInferenceEngine::new(
            KimiCodeConfig {
                api_key: Some("test-key".to_string()),
                base_url: None,
            },
            KimiCodeProviderSpec::default(),
        );
        let metadata = engine.metadata();
        assert_eq!(metadata.auth_type, ProviderAuthType::OAuth);
        assert_eq!(metadata.auth_configured, Some(true));
    }
}