use roder_api::catalog::{HOPPER_DEFAULT_BASE_URL, PROVIDER_HOPPER, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext, ModelDescriptor,
    ProviderAuthType,
};
use roder_ext_openai_chat_completions::{ChatCompletionsRequestConfig, stream_chat_completions};

/// Hopper OpenAI-compatible Chat Completions base URL.
pub const DEFAULT_BASE_URL: &str = HOPPER_DEFAULT_BASE_URL;

/// Provider display name forwarded into transport error messages.
pub const PROVIDER_NAME: &str = "Hopper";

/// Primary, Hopper-documented API-key env var.
pub const API_KEY_ENV: &str = "HOPPER_API_KEY";

/// Roder-prefixed API-key aliases. Never fall back to OPENAI_API_KEY.
pub const API_KEY_ALIASES: &[&str] = &["RODER_HOPPER_API_KEY"];

/// Hopper-specific base-URL env overrides, checked in order.
pub const BASE_URL_ALIASES: &[&str] = &["HOPPER_BASE_URL", "RODER_HOPPER_BASE_URL"];

const SORT_ORDER: i32 = 22;

#[derive(Debug, Clone, Default)]
pub struct HopperConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

pub struct HopperInferenceEngine {
    config: HopperConfig,
}

impl HopperInferenceEngine {
    pub fn new(config: HopperConfig) -> Self {
        Self { config }
    }

    /// Resolve the API key from explicit config, Hopper env vars, then the
    /// `[providers.hopper]` config table. No OpenAI/Anthropic fallbacks.
    pub fn api_key(&self) -> Option<String> {
        nonempty(self.config.api_key.clone())
            .or_else(|| env_nonempty(API_KEY_ENV))
            .or_else(|| first_env_nonempty(API_KEY_ALIASES))
            .or_else(|| config_api_key(PROVIDER_HOPPER))
    }

    /// Resolve the base URL from explicit config, `[providers.hopper]`,
    /// Hopper env overrides, then the documented default. Trailing slashes
    /// are trimmed so `{base}/chat/completions` joins cleanly.
    pub fn base_url(&self) -> String {
        nonempty(self.config.base_url.clone())
            .or_else(|| config_base_url(PROVIDER_HOPPER))
            .or_else(|| first_env_nonempty(BASE_URL_ALIASES))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string()
    }
}

#[async_trait::async_trait]
impl InferenceEngine for HopperInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_HOPPER.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: true,
            structured_output: false,
            image_input: true,
            prompt_cache: true,
            provider_metadata: true,
            tool_search: false,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: PROVIDER_NAME.to_string(),
            description: Some(
                "Hopper OpenAI-compatible Chat Completions provider (api.withhopper.com): fast open-source model inference."
                    .to_string(),
            ),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some(API_KEY_ENV.to_string()),
            auth_configured: Some(self.api_key().is_some()),
            recommended: true,
            sort_order: SORT_ORDER,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_HOPPER, false))
    }

    fn tool_result_image_input(&self, model: &str) -> bool {
        roder_api::catalog::lookup_model_for_provider(PROVIDER_HOPPER, model)
            .is_some_and(|model| model.provider == PROVIDER_HOPPER && model.supports_images)
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let Some(api_key) = self.api_key() else {
            anyhow::bail!(
                "Hopper API key is missing; set {API_KEY_ENV} (or {alias}) or add it under \
                 [providers.hopper] in your Roder config, then retry",
                alias = API_KEY_ALIASES.join(", ")
            )
        };
        let base_url = self.base_url();
        let mut config = ChatCompletionsRequestConfig::bearer(PROVIDER_NAME, base_url, api_key);
        // Hopper serves DeepSeek with DeepSeek's thinking-mode wire format:
        // an explicit `thinking` toggle plus `reasoning_effort`.
        config.include_thinking_param = true;
        stream_chat_completions(config, request).await
    }
}

fn config_api_key(provider_id: &str) -> Option<String> {
    roder_config::load_config().ok().and_then(|config| {
        config.providers.get(provider_id).and_then(|provider| {
            nonempty(provider.api_key.clone())
                .or_else(|| provider.api_key_env.as_deref().and_then(env_nonempty))
        })
    })
}

fn config_base_url(provider_id: &str) -> Option<String> {
    roder_config::load_config().ok().and_then(|config| {
        config
            .providers
            .get(provider_id)
            .and_then(|provider| nonempty(provider.base_url.clone()))
    })
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .and_then(|value| nonempty(Some(value)))
}

fn first_env_nonempty(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| env_nonempty(key))
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestEnv;

    #[test]
    fn tool_result_images_require_a_hopper_catalog_entry() {
        let _env = TestEnv::new();
        let engine = HopperInferenceEngine::new(HopperConfig::default());
        let foreign = roder_api::catalog::built_in_models(false)
            .into_iter()
            .find(|model| model.provider != PROVIDER_HOPPER && model.supports_images)
            .expect("catalog includes a foreign vision model");
        let fallback =
            roder_api::catalog::lookup_model_for_provider(PROVIDER_HOPPER, foreign.id).unwrap();
        assert_ne!(fallback.provider, PROVIDER_HOPPER);
        assert!(fallback.supports_images);
        assert!(!engine.tool_result_image_input(foreign.id));
        assert!(!engine.tool_result_image_input("unknown-model"));
        assert!(engine.tool_result_image_input("deepseek-v4.1-flash-1017"));
        assert!(engine.tool_result_image_input("deepseek-v4.1-flash-1017-abliterated"));
    }

    #[test]
    fn metadata_reports_api_key_auth_state() {
        let _env = TestEnv::new();

        let unauth = HopperInferenceEngine::new(HopperConfig::default());
        let metadata = unauth.metadata();
        assert_eq!(metadata.name, "Hopper");
        assert_eq!(metadata.auth_type, ProviderAuthType::ApiKey);
        assert_eq!(metadata.auth_label.as_deref(), Some("HOPPER_API_KEY"));
        assert_eq!(metadata.auth_configured, Some(false));

        let configured = HopperInferenceEngine::new(HopperConfig {
            api_key: Some("secret".to_string()),
            base_url: None,
        });
        assert_eq!(configured.metadata().auth_configured, Some(true));
    }

    #[test]
    fn base_url_defaults_to_documented_openai_endpoint() {
        let _env = TestEnv::new();
        let engine = HopperInferenceEngine::new(HopperConfig::default());
        assert_eq!(engine.base_url(), "https://api.withhopper.com/v1");

        let custom = HopperInferenceEngine::new(HopperConfig {
            api_key: None,
            base_url: Some("https://api.withhopper.com/v1/".to_string()),
        });
        assert_eq!(custom.base_url(), "https://api.withhopper.com/v1");
    }

    #[tokio::test]
    async fn list_models_returns_built_in_models_without_credentials() {
        let _env = TestEnv::new_async().await;
        let engine = HopperInferenceEngine::new(HopperConfig::default());
        let models = engine
            .list_models(InferenceProviderContext {
                provider_id: PROVIDER_HOPPER,
            })
            .await
            .unwrap();
        assert!(
            models
                .iter()
                .any(|model| model.id == "deepseek-v4.1-flash-1017")
        );
        assert!(
            models
                .iter()
                .any(|model| model.id == "deepseek-v4.1-flash-1017-abliterated")
        );
    }
}
