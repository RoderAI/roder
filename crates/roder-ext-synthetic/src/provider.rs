use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use roder_api::catalog::{PROVIDER_SYNTHETIC, SYNTHETIC_DEFAULT_BASE_URL, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext, ModelDescriptor,
    ProviderAuthType,
};
use roder_ext_openai_chat_completions::{ChatCompletionsRequestConfig, stream_chat_completions};
use roder_ext_openai_responses::{
    cache_ttl, cached_models, discover_models, force_refresh_requested, save_cached_models,
};

/// Synthetic's documented OpenAI-compatible Chat Completions base URL.
pub const DEFAULT_BASE_URL: &str = SYNTHETIC_DEFAULT_BASE_URL;

/// Provider display name forwarded into transport error messages.
pub const PROVIDER_NAME: &str = "Synthetic";

/// Primary, Synthetic-documented API-key env var.
pub const API_KEY_ENV: &str = "SYNTHETIC_API_KEY";

/// Roder-prefixed API-key aliases. We intentionally never read
/// `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` so credentials cannot be sent to
/// the wrong provider by accident.
pub const API_KEY_ALIASES: &[&str] = &["RODER_SYNTHETIC_API_KEY"];

/// Synthetic-specific base-URL env overrides, checked in order.
pub const BASE_URL_ALIASES: &[&str] = &[
    "SYNTHETIC_BASE_URL",
    "SYNTHETIC_OPENAI_BASE_URL",
    "RODER_SYNTHETIC_BASE_URL",
];

const SORT_ORDER: i32 = 20;

#[derive(Debug, Clone, Default)]
pub struct SyntheticConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

pub struct SyntheticInferenceEngine {
    config: SyntheticConfig,
    refresh_in_flight: Arc<AtomicBool>,
}

impl SyntheticInferenceEngine {
    pub fn new(config: SyntheticConfig) -> Self {
        Self {
            config,
            refresh_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Resolve the API key from explicit config, Synthetic env vars, then the
    /// `[providers.synthetic]` config table. No OpenAI/Anthropic fallbacks.
    pub fn api_key(&self) -> Option<String> {
        nonempty(self.config.api_key.clone())
            .or_else(|| env_nonempty(API_KEY_ENV))
            .or_else(|| first_env_nonempty(API_KEY_ALIASES))
            .or_else(|| config_api_key(PROVIDER_SYNTHETIC))
    }

    /// Resolve the base URL from explicit config, `[providers.synthetic]`,
    /// Synthetic env overrides, then the documented default. Trailing slashes
    /// are trimmed so `{base}/chat/completions` joins cleanly.
    pub fn base_url(&self) -> String {
        nonempty(self.config.base_url.clone())
            .or_else(|| config_base_url(PROVIDER_SYNTHETIC))
            .or_else(|| first_env_nonempty(BASE_URL_ALIASES))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string()
    }

    fn schedule_model_refresh(&self, base_url: String, api_key: Option<String>) {
        if self
            .refresh_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let refresh_in_flight = Arc::clone(&self.refresh_in_flight);
        tokio::spawn(async move {
            if let Ok(models) = discover_models(&base_url, api_key.as_deref()).await {
                let _ = save_cached_models(PROVIDER_SYNTHETIC, &base_url, &models);
            }
            refresh_in_flight.store(false, Ordering::Release);
        });
    }
}

#[async_trait::async_trait]
impl InferenceEngine for SyntheticInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_SYNTHETIC.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: false,
            structured_output: true,
            image_input: true,
            prompt_cache: false,
            provider_metadata: true,
            tool_search: false,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: PROVIDER_NAME.to_string(),
            description: Some(
                "Synthetic OpenAI-compatible Chat Completions provider (syn: aliases and hf: model ids)."
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
        let base_url = self.base_url();
        let api_key = self.api_key();
        let cached = cached_models(PROVIDER_SYNTHETIC, &base_url).ok();
        // Discovery only happens with a key and is always scheduled in the
        // background so app-server/TUI provider lists never block on network.
        if api_key.is_some() {
            let should_refresh = force_refresh_requested()
                || cached
                    .as_ref()
                    .map(|entry| entry.is_stale(cache_ttl()))
                    .unwrap_or(true);
            if should_refresh {
                self.schedule_model_refresh(base_url.clone(), api_key.clone());
            }
        }
        if let Some(entry) = cached
            && !entry.models.is_empty()
        {
            return Ok(entry.models);
        }
        Ok(models_for_provider(PROVIDER_SYNTHETIC, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        // Fail before any network access with concise setup guidance.
        let Some(api_key) = self.api_key() else {
            anyhow::bail!(
                "Synthetic API key is missing; set {API_KEY_ENV} (or {alias}) or add it under \
                 [providers.synthetic] in your Roder config, then retry",
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
            "roder-ext-synthetic-tests-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        // SAFETY: set once before any test reads the config; all tests run in
        // the same process and never restore a real config dir.
        unsafe { std::env::set_var("RODER_CONFIG_DIR", &temp); }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_reports_api_key_auth_state() {
        isolate_config_dir();

        let unauth = SyntheticInferenceEngine::new(SyntheticConfig::default());
        let metadata = unauth.metadata();
        assert_eq!(metadata.name, "Synthetic");
        assert_eq!(metadata.auth_type, ProviderAuthType::ApiKey);
        assert_eq!(metadata.auth_label.as_deref(), Some("SYNTHETIC_API_KEY"));
        assert_eq!(metadata.auth_configured, Some(false));

        let configured = SyntheticInferenceEngine::new(SyntheticConfig {
            api_key: Some("secret".to_string()),
            base_url: None,
        });
        assert_eq!(configured.metadata().auth_configured, Some(true));
    }

    #[test]
    fn base_url_defaults_to_documented_openai_endpoint() {
        let engine = SyntheticInferenceEngine::new(SyntheticConfig::default());
        assert_eq!(engine.base_url(), "https://api.synthetic.new/openai/v1");

        let custom = SyntheticInferenceEngine::new(SyntheticConfig {
            api_key: None,
            base_url: Some("https://api.synthetic.new/v1/".to_string()),
        });
        assert_eq!(custom.base_url(), "https://api.synthetic.new/v1");
    }

    #[tokio::test]
    async fn list_models_returns_alias_fallback_without_credentials() {
        isolate_config_dir();
        let engine = SyntheticInferenceEngine::new(SyntheticConfig::default());
        let models = engine
            .list_models(InferenceProviderContext {
                provider_id: PROVIDER_SYNTHETIC,
            })
            .await
            .unwrap();
        assert!(models.iter().any(|model| model.id == "syn:large:text"));
        assert!(models.iter().any(|model| model.id == "syn:small:text"));
        assert!(models.iter().any(|model| model.id == "syn:large:vision"));
        assert!(models.iter().any(|model| model.id == "syn:small:vision"));
    }

    #[tokio::test]
    async fn list_models_returns_always_on_hf_models_without_credentials() {
        isolate_config_dir();
        let engine = SyntheticInferenceEngine::new(SyntheticConfig::default());
        let models = engine
            .list_models(InferenceProviderContext {
                provider_id: PROVIDER_SYNTHETIC,
            })
            .await
            .unwrap();
        let glm_5_2 = models
            .iter()
            .find(|model| model.id == "hf:zai-org/GLM-5.2")
            .expect("GLM-5.2 always-on model should be listed");
        assert_eq!(
            glm_5_2.context_window,
            Some(524_288),
            "GLM-5.2 should advertise its documented 512k context"
        );
        let minimax = models
            .iter()
            .find(|model| model.id == "hf:MiniMaxAI/MiniMax-M3")
            .expect("MiniMax-M3 always-on model should be listed");
        assert_eq!(minimax.context_window, Some(524_288));
        for id in [
            "hf:Qwen/Qwen3.6-27B",
            "hf:moonshotai/Kimi-K2.6",
            "hf:nvidia/NVIDIA-Nemotron-3-Super-120B-A12B-NVFP4",
            "hf:zai-org/GLM-4.7",
            "hf:zai-org/GLM-4.7-Flash",
            "hf:zai-org/GLM-5.1",
            "hf:openai/gpt-oss-120b",
            "hf:Qwen/Qwen3.5-397B-A17B",
        ] {
            assert!(
                models.iter().any(|model| model.id == id),
                "always-on model {id} should be listed offline"
            );
        }
    }
}
