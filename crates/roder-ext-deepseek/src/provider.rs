use roder_api::catalog::{
    DEEPSEEK_DEFAULT_BASE_URL, PROVIDER_DEEPSEEK, models_for_provider,
};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext, ModelDescriptor,
    ProviderAuthType,
};
use roder_ext_openai_chat_completions::{ChatCompletionsRequestConfig, stream_chat_completions};

/// DeepSeek Platform OpenAI-compatible Chat Completions base URL.
pub const DEFAULT_BASE_URL: &str = DEEPSEEK_DEFAULT_BASE_URL;

/// Provider display name forwarded into transport error messages.
pub const PROVIDER_NAME: &str = "DeepSeek Platform";

/// Primary, DeepSeek-documented API-key env var.
pub const API_KEY_ENV: &str = "DEEPSEEK_API_KEY";

/// Roder-prefixed API-key aliases. Never fall back to OPENAI_API_KEY.
pub const API_KEY_ALIASES: &[&str] = &["RODER_DEEPSEEK_API_KEY"];

/// DeepSeek-specific base-URL env overrides, checked in order.
pub const BASE_URL_ALIASES: &[&str] = &["DEEPSEEK_BASE_URL", "RODER_DEEPSEEK_BASE_URL"];

const SORT_ORDER: i32 = 21;

#[derive(Debug, Clone, Default)]
pub struct DeepSeekConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

pub struct DeepSeekInferenceEngine {
    config: DeepSeekConfig,
}

impl DeepSeekInferenceEngine {
    pub fn new(config: DeepSeekConfig) -> Self {
        Self { config }
    }

    /// Resolve the API key from explicit config, DeepSeek env vars, then the
    /// `[providers.deepseek]` config table. No OpenAI/Anthropic fallbacks.
    pub fn api_key(&self) -> Option<String> {
        nonempty(self.config.api_key.clone())
            .or_else(|| env_nonempty(API_KEY_ENV))
            .or_else(|| first_env_nonempty(API_KEY_ALIASES))
            .or_else(|| config_api_key(PROVIDER_DEEPSEEK))
    }

    /// Resolve the base URL from explicit config, `[providers.deepseek]`,
    /// DeepSeek env overrides, then the documented default. Trailing slashes
    /// are trimmed so `{base}/chat/completions` joins cleanly.
    pub fn base_url(&self) -> String {
        nonempty(self.config.base_url.clone())
            .or_else(|| config_base_url(PROVIDER_DEEPSEEK))
            .or_else(|| first_env_nonempty(BASE_URL_ALIASES))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string()
    }
}

#[async_trait::async_trait]
impl InferenceEngine for DeepSeekInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_DEEPSEEK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: false,
            structured_output: true,
            image_input: false,
            prompt_cache: true,
            provider_metadata: true,
            tool_search: false,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: PROVIDER_NAME.to_string(),
            description: Some(
                "DeepSeek Platform OpenAI-compatible Chat Completions provider (api.deepseek.com)."
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
        Ok(models_for_provider(PROVIDER_DEEPSEEK, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let Some(api_key) = self.api_key() else {
            anyhow::bail!(
                "DeepSeek API key is missing; set {API_KEY_ENV} (or {alias}) or add it under \
                 [providers.deepseek] in your Roder config, then retry",
                alias = API_KEY_ALIASES.join(", ")
            )
        };
        let base_url = self.base_url();
        stream_chat_completions(
            ChatCompletionsRequestConfig::bearer(PROVIDER_NAME, base_url, api_key),
            request,
        )
        .await
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
fn isolate_config_dir() {
    use std::sync::OnceLock;
    static CONFIG_ISOLATION: OnceLock<()> = OnceLock::new();
    CONFIG_ISOLATION.get_or_init(|| {
        let temp = std::env::temp_dir().join(format!(
            "roder-ext-deepseek-tests-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        // SAFETY: set once before any test reads the config; all tests run in
        // the same process and never restore a real config dir.
        unsafe {
            std::env::set_var("RODER_CONFIG_DIR", &temp);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_reports_api_key_auth_state() {
        isolate_config_dir();

        let unauth = DeepSeekInferenceEngine::new(DeepSeekConfig::default());
        let metadata = unauth.metadata();
        assert_eq!(metadata.name, "DeepSeek Platform");
        assert_eq!(metadata.auth_type, ProviderAuthType::ApiKey);
        assert_eq!(metadata.auth_label.as_deref(), Some("DEEPSEEK_API_KEY"));
        assert_eq!(metadata.auth_configured, Some(false));

        let configured = DeepSeekInferenceEngine::new(DeepSeekConfig {
            api_key: Some("secret".to_string()),
            base_url: None,
        });
        assert_eq!(configured.metadata().auth_configured, Some(true));
    }

    #[test]
    fn base_url_defaults_to_documented_openai_endpoint() {
        let engine = DeepSeekInferenceEngine::new(DeepSeekConfig::default());
        assert_eq!(engine.base_url(), "https://api.deepseek.com/v1");

        let custom = DeepSeekInferenceEngine::new(DeepSeekConfig {
            api_key: None,
            base_url: Some("https://api.deepseek.com/".to_string()),
        });
        assert_eq!(custom.base_url(), "https://api.deepseek.com");
    }

    #[tokio::test]
    async fn list_models_returns_built_in_models_without_credentials() {
        isolate_config_dir();
        let engine = DeepSeekInferenceEngine::new(DeepSeekConfig::default());
        let models = engine
            .list_models(InferenceProviderContext {
                provider_id: PROVIDER_DEEPSEEK,
            })
            .await
            .unwrap();
        assert!(models.iter().any(|model| model.id == "deepseek-chat"));
        assert!(models.iter().any(|model| model.id == "deepseek-reasoner"));
        assert!(models.iter().any(|model| model.id == "deepseek-v4-flash"));
        assert!(models.iter().any(|model| model.id == "deepseek-v4-pro"));
    }
}
