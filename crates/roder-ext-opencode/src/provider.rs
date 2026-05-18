use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use roder_api::catalog::{PROVIDER_OPENCODE, PROVIDER_OPENCODE_GO, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext, ModelDescriptor,
    ProviderAuthType,
};
use roder_ext_openai_responses::OpenAiResponsesEngine;
use serde::{Deserialize, Serialize};

const DEFAULT_ZEN_BASE_URL: &str = "https://opencode.ai/zen/v1";
const DEFAULT_GO_BASE_URL: &str = "https://opencode.ai/zen/go/v1";
const DEFAULT_MODELS_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Default)]
pub struct OpenCodeConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub project_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OpenCodeProviderSpec {
    pub provider_id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub default_base_url: &'static str,
    pub sort_order: i32,
    pub api_key_env: &'static str,
    pub api_key_aliases: &'static [&'static str],
    pub base_url_env: &'static str,
    pub base_url_aliases: &'static [&'static str],
    refresh_in_flight: Arc<AtomicBool>,
}

impl OpenCodeProviderSpec {
    pub fn zen() -> Self {
        Self {
            provider_id: PROVIDER_OPENCODE,
            name: "OpenCode Zen",
            description: "OpenCode Zen subscription API key provider",
            default_base_url: DEFAULT_ZEN_BASE_URL,
            sort_order: 15,
            api_key_env: "OPENCODE_API_KEY",
            api_key_aliases: &["RODER_OPENCODE_API_KEY", "OPENCODE_ZEN_API_KEY"],
            base_url_env: "RODER_OPENCODE_BASE_URL",
            base_url_aliases: &["OPENCODE_BASE_URL", "OPENCODE_ZEN_BASE_URL"],
            refresh_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn go() -> Self {
        Self {
            provider_id: PROVIDER_OPENCODE_GO,
            name: "OpenCode Go",
            description: "OpenCode Go subscription API key provider",
            default_base_url: DEFAULT_GO_BASE_URL,
            sort_order: 16,
            api_key_env: "OPENCODE_GO_API_KEY",
            api_key_aliases: &["RODER_OPENCODE_GO_API_KEY", "OPENCODE_API_KEY"],
            base_url_env: "RODER_OPENCODE_GO_BASE_URL",
            base_url_aliases: &["OPENCODE_GO_BASE_URL"],
            refresh_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub struct OpenCodeInferenceEngine {
    config: OpenCodeConfig,
    spec: OpenCodeProviderSpec,
}

impl OpenCodeInferenceEngine {
    pub fn new(config: OpenCodeConfig, spec: OpenCodeProviderSpec) -> Self {
        Self { config, spec }
    }

    fn base_url(&self) -> String {
        nonempty(self.config.base_url.clone())
            .or_else(|| config_base_url(self.spec.provider_id))
            .or_else(|| env_nonempty(self.spec.base_url_env))
            .or_else(|| first_env_nonempty(self.spec.base_url_aliases))
            .unwrap_or_else(|| self.spec.default_base_url.to_string())
            .trim_end_matches('/')
            .to_string()
    }

    fn api_key(&self) -> Option<String> {
        nonempty(self.config.api_key.clone())
            .or_else(|| env_nonempty(self.spec.api_key_env))
            .or_else(|| first_env_nonempty(self.spec.api_key_aliases))
            .or_else(|| config_api_key(self.spec.provider_id))
    }

    fn project_id(&self) -> Option<String> {
        nonempty(self.config.project_id.clone())
            .or_else(|| config_project_id(self.spec.provider_id))
    }

    fn request_headers(&self, ctx: &InferenceTurnContext<'_>) -> Vec<(String, String)> {
        let mut headers = vec![
            ("x-opencode-client".to_string(), "roder".to_string()),
            (
                "User-Agent".to_string(),
                format!("roder/{}", env!("CARGO_PKG_VERSION")),
            ),
        ];
        if !ctx.thread_id.is_empty() {
            headers.push(("x-opencode-session".to_string(), ctx.thread_id.to_string()));
        }
        if !ctx.turn_id.is_empty() {
            headers.push(("x-opencode-request".to_string(), ctx.turn_id.to_string()));
        }
        if let Some(project_id) = self.project_id() {
            headers.push(("x-opencode-project".to_string(), project_id));
        }
        headers
    }

    fn schedule_model_refresh(&self, base_url: String, api_key: Option<String>) {
        let provider_id = self.spec.provider_id.to_string();
        let refresh_in_flight = Arc::clone(&self.spec.refresh_in_flight);
        if refresh_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        tokio::spawn(async move {
            let result = discover_models(base_url.clone(), api_key).await;
            if let Ok(models) = result {
                let _ = save_cached_models(&provider_id, &base_url, &models);
            }
            refresh_in_flight.store(false, Ordering::Release);
        });
    }
}

#[async_trait::async_trait]
impl InferenceEngine for OpenCodeInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        self.spec.provider_id.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: true,
            structured_output: true,
            image_input: false,
            prompt_cache: true,
            provider_metadata: true,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: self.spec.name.to_string(),
            description: Some(self.spec.description.to_string()),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some("API key".to_string()),
            auth_configured: Some(self.api_key().is_some()),
            recommended: true,
            sort_order: self.spec.sort_order,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        let base_url = self.base_url();
        let cached = cached_models(self.spec.provider_id, &base_url).ok();
        let should_refresh = force_refresh_requested()
            || cached
                .as_ref()
                .map(|entry| entry.is_stale(cache_ttl()))
                .unwrap_or(true);

        if should_refresh {
            self.schedule_model_refresh(base_url, self.api_key());
        }

        if let Some(entry) = cached {
            if !entry.models.is_empty() {
                return Ok(entry.models);
            }
        }

        Ok(models_for_provider(self.spec.provider_id, false))
    }

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let Some(api_key) = self.api_key() else {
            anyhow::bail!(
                "{} API key is missing; set {} or configure it from the provider menu",
                self.spec.name,
                self.spec.api_key_env
            )
        };
        OpenAiResponsesEngine::new_with_config(
            api_key,
            self.spec.provider_id,
            self.base_url(),
            self.request_headers(&ctx),
        )
        .stream_turn(ctx, request)
        .await
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ModelsCacheFile {
    #[serde(default)]
    providers: BTreeMap<String, CachedProviderModels>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedProviderModels {
    fetched_at: u64,
    base_url: String,
    models: Vec<ModelDescriptor>,
}

impl CachedProviderModels {
    fn is_stale(&self, ttl: Duration) -> bool {
        ttl.is_zero()
            || now_unix_secs()
                .saturating_sub(self.fetched_at)
                .ge(&ttl.as_secs())
    }
}

async fn discover_models(
    base_url: String,
    api_key: Option<String>,
) -> anyhow::Result<Vec<ModelDescriptor>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    let mut request = client.get(format!("{}/models", base_url.trim_end_matches('/')));
    if let Some(api_key) = api_key {
        request = request.bearer_auth(api_key);
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        anyhow::bail!("OpenCode model discovery failed: {}", response.status());
    }
    let body: ModelsResponse = response.json().await?;
    let models = body
        .data
        .into_iter()
        .filter_map(|model| {
            let id = model.id.trim();
            (!id.is_empty()).then(|| ModelDescriptor {
                id: id.to_string(),
                name: model.name.unwrap_or_else(|| id.to_string()),
                context_window: None,
                default_reasoning: None,
                supported_reasoning: Vec::new(),
            })
        })
        .collect::<Vec<_>>();
    if models.is_empty() {
        anyhow::bail!("OpenCode model discovery returned no models");
    }
    Ok(models)
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

fn config_project_id(provider_id: &str) -> Option<String> {
    roder_config::load_config().ok().and_then(|config| {
        config.providers.get(provider_id).and_then(|provider| {
            nonempty(provider.project_id.clone())
                .or_else(|| provider.project_id_env.as_deref().and_then(env_nonempty))
        })
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

fn cached_models(provider_id: &str, base_url: &str) -> anyhow::Result<CachedProviderModels> {
    let cache: ModelsCacheFile = serde_json::from_str(&fs::read_to_string(cache_path())?)?;
    let entry = cache
        .providers
        .get(provider_id)
        .filter(|entry| entry.base_url.trim_end_matches('/') == base_url.trim_end_matches('/'))
        .cloned();
    entry.ok_or_else(|| anyhow::anyhow!("no cached models for {provider_id}"))
}

fn save_cached_models(
    provider_id: &str,
    base_url: &str,
    models: &[ModelDescriptor],
) -> anyhow::Result<()> {
    let path = cache_path();
    let mut cache = fs::read_to_string(&path)
        .ok()
        .and_then(|body| serde_json::from_str::<ModelsCacheFile>(&body).ok())
        .unwrap_or_default();
    cache.providers.insert(
        provider_id.to_string(),
        CachedProviderModels {
            fetched_at: now_unix_secs(),
            base_url: base_url.trim_end_matches('/').to_string(),
            models: models.to_vec(),
        },
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(&cache)?)?;
    Ok(())
}

fn cache_path() -> PathBuf {
    if let Some(path) = env_nonempty("RODER_MODELS_CACHE_PATH") {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".roder")
        .join("models-cache.json")
}

fn cache_ttl() -> Duration {
    env_nonempty("RODER_MODELS_CACHE_TTL_SECONDS")
        .or_else(|| env_nonempty("RODER_OPENCODE_MODELS_CACHE_TTL_SECONDS"))
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_MODELS_CACHE_TTL)
}

fn force_refresh_requested() -> bool {
    env_nonempty("RODER_MODELS_REFRESH")
        .or_else(|| env_nonempty("RODER_OPENCODE_MODELS_REFRESH"))
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::inference::InferenceEngine;

    #[test]
    fn opencode_metadata_uses_single_provider_id() {
        let engine =
            OpenCodeInferenceEngine::new(OpenCodeConfig::default(), OpenCodeProviderSpec::zen());
        assert_eq!(engine.id(), "opencode");
        assert_eq!(engine.metadata().name, "OpenCode Zen");
        let go =
            OpenCodeInferenceEngine::new(OpenCodeConfig::default(), OpenCodeProviderSpec::go());
        assert_eq!(go.id(), "opencode-go");
        assert_eq!(go.metadata().name, "OpenCode Go");
    }

    #[tokio::test]
    async fn list_models_returns_cached_models_without_waiting_for_refresh() {
        let cache_file = std::env::temp_dir().join(format!(
            "roder-opencode-models-cache-{}-{}.json",
            std::process::id(),
            now_unix_secs()
        ));
        let model_id = "cached-opencode-model";
        let base_url = "http://127.0.0.1:9";
        let cached_model = ModelDescriptor {
            id: model_id.to_string(),
            name: "Cached OpenCode Model".to_string(),
            context_window: None,
            default_reasoning: None,
            supported_reasoning: Vec::new(),
        };

        unsafe {
            std::env::set_var("RODER_MODELS_CACHE_PATH", &cache_file);
            std::env::set_var("RODER_OPENCODE_MODELS_CACHE_TTL_SECONDS", "0");
        }
        save_cached_models(
            PROVIDER_OPENCODE,
            base_url,
            std::slice::from_ref(&cached_model),
        )
        .unwrap();
        let engine = OpenCodeInferenceEngine::new(
            OpenCodeConfig {
                base_url: Some(base_url.to_string()),
                ..OpenCodeConfig::default()
            },
            OpenCodeProviderSpec::zen(),
        );

        let models = tokio::time::timeout(
            Duration::from_millis(100),
            engine.list_models(InferenceProviderContext {
                provider_id: PROVIDER_OPENCODE,
            }),
        )
        .await
        .expect("cached model listing should not wait for background refresh")
        .unwrap();

        assert_eq!(models, vec![cached_model]);
        unsafe {
            std::env::remove_var("RODER_MODELS_CACHE_PATH");
            std::env::remove_var("RODER_OPENCODE_MODELS_CACHE_TTL_SECONDS");
        }
        let _ = fs::remove_file(cache_file);
    }
}
