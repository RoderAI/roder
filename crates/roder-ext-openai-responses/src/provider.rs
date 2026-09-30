#[path = "native_compaction.rs"]
mod native_compaction;
use roder_api::provider_error::{ProviderFailure, ProviderFailureKind};
#[path = "response_transport.rs"]
mod response_transport;
use response_transport::*;
#[path = "request_budget.rs"]
mod request_budget;
use request_budget::*;
#[path = "response_websocket.rs"]
mod response_websocket;
use response_websocket::*;
#[path = "tool_definitions.rs"]
mod tool_definitions;
use tool_definitions::*;
pub use tool_definitions::{
    openai_model_supports_freeform_apply_patch, openai_model_supports_tool_search,
};
#[path = "response_stream.rs"]
mod response_stream;
use response_stream::*;
#[path = "client_search.rs"]
mod client_search;
use client_search::*;
#[path = "response_events.rs"]
mod response_events;
#[path = "response_replay.rs"]
mod response_replay;
use response_events::*;
use response_replay::*;
#[path = "response_instructions.rs"]
mod response_instructions;
use response_instructions::*;
#[path = "response_tools.rs"]
mod response_tools;
use response_tools::*;

use crate::stream_diagnostics::ResponseStreamDiagnostics;
use roder_api::catalog::{
    PROVIDER_CODEX, PROVIDER_FIREWORKS, PROVIDER_OPENAI, PROVIDER_OPENROUTER, PROVIDER_SUPERGROK,
    PROVIDER_XAI, REASONING_MAX, REASONING_ULTRA, lookup_model, lookup_model_for_provider,
    models_for_provider,
};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::CompactionProgress;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, HostedToolCallCompleted, HostedToolCallStarted,
    HostedWebSearchMode, InferenceCapabilities, InferenceEngine, InferenceEvent,
    InferenceEventStream, InferenceFailure, InferenceProviderContext, InferenceProviderMetadata,
    InferenceTurnContext, MessageDelta, ModelDescriptor, ProviderAuthType, ReasoningDelta,
    TokenUsage, ToolCallCompleted, ToolCallDelta, ToolCallStarted,
};
use roder_api::reliability::{
    ReliabilityRequestPolicy, provider_retry_delay_ms, provider_retry_metadata,
    provider_retry_status_cause,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FINAL_ANSWER_PHASE: &str = "final_answer";
const DEFAULT_MODELS_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const DEFAULT_RESPONSES_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const RESPONSES_STREAM_IDLE_TIMEOUT_ENV: &str = "RODER_RESPONSES_STREAM_IDLE_TIMEOUT_MS";

pub struct OpenAiResponsesEngine {
    api_key: Option<String>,
    provider_id: String,
    display_name: String,
    base_url: String,
    headers: Vec<(String, String)>,
    profile: ResponsesProviderProfile,
    discover_models: bool,
    refresh_in_flight: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponsesProviderProfile {
    OpenAi,
    Xai,
    OpenRouter,
    Fireworks,
}

impl OpenAiResponsesEngine {
    pub fn new(api_key: Option<String>) -> Self {
        Self::new_with_provider_id(api_key, PROVIDER_OPENAI)
    }

    pub fn new_with_provider_id(api_key: Option<String>, provider_id: impl Into<String>) -> Self {
        Self::new_with_config(
            api_key,
            provider_id,
            "https://api.openai.com/v1",
            Vec::new(),
        )
    }

    pub fn new_with_config(
        api_key: Option<String>,
        provider_id: impl Into<String>,
        base_url: impl Into<String>,
        headers: Vec<(String, String)>,
    ) -> Self {
        let provider_id = provider_id.into();
        let profile = match provider_id.as_str() {
            PROVIDER_XAI | PROVIDER_SUPERGROK => ResponsesProviderProfile::Xai,
            PROVIDER_OPENROUTER => ResponsesProviderProfile::OpenRouter,
            PROVIDER_FIREWORKS => ResponsesProviderProfile::Fireworks,
            _ => ResponsesProviderProfile::OpenAi,
        };
        let display_name = match provider_id.as_str() {
            PROVIDER_XAI => "xAI".to_string(),
            PROVIDER_OPENROUTER => "OpenRouter".to_string(),
            PROVIDER_FIREWORKS => "Fireworks AI".to_string(),
            _ => "OpenAI".to_string(),
        };
        Self {
            api_key,
            provider_id,
            display_name,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            headers,
            profile,
            discover_models: false,
            refresh_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn new_openrouter_provider(
        api_key: Option<String>,
        base_url: impl Into<String>,
        headers: Vec<(String, String)>,
    ) -> Self {
        Self {
            api_key,
            provider_id: PROVIDER_OPENROUTER.to_string(),
            display_name: "OpenRouter".to_string(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            headers,
            profile: ResponsesProviderProfile::OpenRouter,
            discover_models: true,
            refresh_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn new_fireworks_provider(api_key: Option<String>, base_url: impl Into<String>) -> Self {
        Self {
            api_key,
            provider_id: PROVIDER_FIREWORKS.to_string(),
            display_name: "Fireworks AI".to_string(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            headers: Vec::new(),
            profile: ResponsesProviderProfile::Fireworks,
            discover_models: true,
            refresh_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn new_custom_provider(
        api_key: Option<String>,
        provider_id: impl Into<String>,
        display_name: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            api_key,
            provider_id: provider_id.into(),
            display_name: display_name.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            headers: Vec::new(),
            profile: ResponsesProviderProfile::OpenAi,
            discover_models: true,
            refresh_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    fn schedule_model_refresh(&self) {
        if self
            .refresh_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let provider_id = self.provider_id.clone();
        let base_url = self.base_url.clone();
        let api_key = self.api_key.clone();
        let refresh_in_flight = Arc::clone(&self.refresh_in_flight);
        tokio::spawn(async move {
            let result = discover_models(&base_url, api_key.as_deref()).await;
            if let Ok(models) = result {
                let _ = save_cached_models(&provider_id, &base_url, &models);
            }
            refresh_in_flight.store(false, Ordering::Release);
        });
    }

    /**
     * Map a canonical inference request to the OpenAI Responses request body.
     * Public so offline eval harnesses can snapshot exact provider payloads
     * (for example explicit vs provider-native tool-search request bodies).
     */
    pub fn map_request(request: &AgentInferenceRequest) -> Value {
        Self::map_request_with_options(request, RequestMappingOptions::default()).0
    }

    fn map_request_with_options(
        request: &AgentInferenceRequest,
        options: RequestMappingOptions<'_>,
    ) -> (Value, ResponsesToolNameMap) {
        let (tools, tool_name_map) = responses_tools(request, options.profile);
        let supports_images = if matches!(options.profile, ResponsesProviderProfile::Xai) {
            lookup_model_for_provider(&request.model.provider, &request.model.model)
                .map(|entry| entry.supports_images)
                .unwrap_or(true)
        } else {
            true
        };
        let input = response_input_items_with_options(
            request,
            &tool_name_map,
            options.profile,
            supports_images,
        );
        let mut body = json!({
            "model": request.model.model,
            "input": input,
            "store": false,
            "stream": true,
        });
        // OpenAI + xAI/SuperGrok accept top-level `instructions`. OpenRouter and
        // Fireworks instead receive system/developer text as leading input
        // messages (see response_input_items_with_options). Stable system +
        // developer content must both reach the wire here — ultra mode, plan
        // mode, goals, and agent-control policy all live in `developer`.
        if !matches!(
            options.profile,
            ResponsesProviderProfile::OpenRouter | ResponsesProviderProfile::Fireworks
        ) && let Some(instructions) = stable_responses_instructions(request)
        {
            body["instructions"] = json!(instructions);
        }
        if let Some(max_tokens) = request.output.max_tokens {
            body["max_output_tokens"] = json!(max_tokens);
        }
        if let Some(temperature) = request.output.temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(top_p) = request.output.top_p {
            body["top_p"] = json!(top_p);
        }
        if let Some(format) = request.output.response_format.as_ref() {
            body["text"] = json!({ "format": format });
        }
        if request.reasoning.enabled {
            match options.profile {
                ResponsesProviderProfile::Xai if xai_supports_reasoning(&request.model.model) => {
                    if let Some(level) = request
                        .reasoning
                        .level
                        .as_deref()
                        .filter(|level| *level != "none")
                    {
                        body["reasoning"] = json!({ "effort": level });
                    }
                }
                ResponsesProviderProfile::OpenRouter => {
                    if let Some(level) = request
                        .reasoning
                        .level
                        .as_deref()
                        .filter(|level| *level != "none")
                    {
                        body["reasoning"] = json!({ "effort": level });
                    }
                }
                ResponsesProviderProfile::OpenAi => {
                    body["reasoning"] = match request.reasoning.level.as_deref() {
                        Some(level) => json!({
                            "effort": openai_reasoning_effort_for_request(level),
                            "summary": "auto"
                        }),
                        None => json!({ "summary": "auto" }),
                    };
                    body["include"] = json!(["reasoning.encrypted_content"]);
                }
                ResponsesProviderProfile::Xai | ResponsesProviderProfile::Fireworks => {}
            }
        }
        // Only OpenAI's own Responses API takes `service_tier` (Fast mode is
        // `"priority"`); OpenRouter, xAI, and Fireworks never receive it.
        if options.profile == ResponsesProviderProfile::OpenAi
            && let Some(service_tier) = request
                .runtime
                .service_tier
                .as_deref()
                .filter(|tier| !tier.is_empty())
        {
            body["service_tier"] = json!(service_tier);
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
            body["tool_choice"] = match &request.tool_choice {
                roder_api::tools::ToolChoice::None
                    if request.tools.is_empty()
                        && request.runtime.hosted_web_search.is_enabled() =>
                {
                    json!("auto")
                }
                roder_api::tools::ToolChoice::None => json!("none"),
                roder_api::tools::ToolChoice::Specific(name) => {
                    let name = tool_name_map.api_name(name);
                    json!({ "type": "function", "name": name })
                }
                roder_api::tools::ToolChoice::Auto | roder_api::tools::ToolChoice::Any => {
                    json!("auto")
                }
            };
            if !request.tools.is_empty() {
                body["parallel_tool_calls"] =
                    json!(request.runtime.parallel_tool_calls.unwrap_or(true));
            }
        }
        let prompt_cache_key = match options.profile {
            ResponsesProviderProfile::Xai => {
                options.thread_id.filter(|thread_id| !thread_id.is_empty())
            }
            ResponsesProviderProfile::Fireworks => None,
            ResponsesProviderProfile::OpenAi | ResponsesProviderProfile::OpenRouter => {
                request.runtime.prompt_cache_key.as_deref()
            }
        };
        if let Some(prompt_cache_key) = prompt_cache_key {
            body["prompt_cache_key"] = json!(prompt_cache_key);
        }
        if options.profile != ResponsesProviderProfile::Fireworks
            && let Some(threshold) = request
                .runtime
                .auto_compact_token_limit
                .filter(|threshold| *threshold > 0)
        {
            body["context_management"] =
                json!([{ "type": "compaction", "compact_threshold": threshold }]);
        }
        (body, tool_name_map)
    }
}

fn openai_reasoning_effort_for_request(effort: &str) -> &str {
    if effort == REASONING_ULTRA {
        REASONING_MAX
    } else {
        effort
    }
}

#[derive(Debug, Clone, Copy)]
struct RequestMappingOptions<'a> {
    profile: ResponsesProviderProfile,
    thread_id: Option<&'a str>,
}

impl Default for RequestMappingOptions<'_> {
    fn default() -> Self {
        Self {
            profile: ResponsesProviderProfile::OpenAi,
            thread_id: None,
        }
    }
}

#[derive(Debug, Default)]
struct ResponsesToolNameMap {
    tool_name_to_api_name: HashMap<String, String>,
    api_name_to_tool_name: HashMap<String, String>,
}

impl ResponsesToolNameMap {
    fn register(&mut self, tool_name: &str, api_name: &str) {
        self.tool_name_to_api_name
            .insert(tool_name.to_string(), api_name.to_string());
        self.api_name_to_tool_name
            .insert(api_name.to_string(), tool_name.to_string());
    }

    fn api_name<'a>(&'a self, tool_name: &'a str) -> &'a str {
        self.tool_name_to_api_name
            .get(tool_name)
            .map_or(tool_name, String::as_str)
    }

    fn replay_api_name(&self, tool_name: &str) -> String {
        self.tool_name_to_api_name
            .get(tool_name)
            .cloned()
            .unwrap_or_else(|| responses_api_tool_name(tool_name))
    }
}

fn xai_supports_reasoning(model: &str) -> bool {
    model == "grok-4.6"
        || model == "grok-4.3"
        || model == "grok-4.20-0309-reasoning"
        || model.starts_with("grok-4.20-multi-agent-")
}

#[derive(Debug, Deserialize)]
pub struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ModelEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ModelsCacheFile {
    #[serde(default)]
    providers: BTreeMap<String, CachedProviderModels>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedProviderModels {
    pub fetched_at: u64,
    pub base_url: String,
    pub models: Vec<ModelDescriptor>,
}

impl CachedProviderModels {
    pub fn is_stale(&self, ttl: Duration) -> bool {
        ttl.is_zero()
            || now_unix_secs()
                .saturating_sub(self.fetched_at)
                .ge(&ttl.as_secs())
    }
}

pub async fn discover_models(
    base_url: &str,
    api_key: Option<&str>,
) -> anyhow::Result<Vec<ModelDescriptor>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    let mut last_error = None;
    for url in model_discovery_urls(base_url) {
        let mut request = client.get(&url);
        if let Some(api_key) = api_key {
            request = request.bearer_auth(api_key);
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                let body: ModelsResponse = response.json().await?;
                let models = models_from_response(body);
                if !models.is_empty() {
                    return Ok(models);
                }
                last_error = Some(anyhow::anyhow!(
                    "model discovery returned no models at {url}"
                ));
            }
            Ok(response) => {
                last_error = Some(anyhow::anyhow!(
                    "model discovery failed at {url}: {}",
                    response.status()
                ));
            }
            Err(err) => {
                last_error = Some(anyhow::anyhow!("model discovery failed at {url}: {err}"));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("model discovery failed")))
}

fn model_discovery_urls(base_url: &str) -> Vec<String> {
    let base = base_url.trim_end_matches('/');
    vec![format!("{base}/models"), format!("{base}/v1/models")]
}

fn models_from_response(body: ModelsResponse) -> Vec<ModelDescriptor> {
    body.data
        .into_iter()
        .filter_map(|model| {
            let id = model.id.trim();
            if id.is_empty() {
                return None;
            }
            // Prefer built-in catalog metadata (context window + reasoning
            // ladder including max/ultra) when we already know the model id;
            // discovery responses never carry effort menus.
            if let Some(catalog) = lookup_model(id) {
                return Some(ModelDescriptor::from(catalog));
            }
            Some(ModelDescriptor {
                id: id.to_string(),
                name: model.name.unwrap_or_else(|| id.to_string()),
                context_window: model.context_length,
                default_reasoning: None,
                supported_reasoning: Vec::new(),
            })
        })
        .collect()
}

pub fn cached_models(provider_id: &str, base_url: &str) -> anyhow::Result<CachedProviderModels> {
    let cache: ModelsCacheFile = serde_json::from_str(&fs::read_to_string(cache_path())?)?;
    cache
        .providers
        .get(provider_id)
        .filter(|entry| entry.base_url.trim_end_matches('/') == base_url.trim_end_matches('/'))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no cached models for {provider_id}"))
}

pub fn save_cached_models(
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
    roder_data_dir().join("models-cache.json")
}

fn roder_data_dir() -> PathBuf {
    std::env::var_os("RODER_DATA_DIR")
        .or_else(|| std::env::var_os("RODER_CONFIG_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".roder")
        })
}

pub fn cache_ttl() -> Duration {
    env_nonempty("RODER_MODELS_CACHE_TTL_SECONDS")
        .or_else(|| env_nonempty("RODER_OPENCODE_MODELS_CACHE_TTL_SECONDS"))
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_MODELS_CACHE_TTL)
}

pub fn force_refresh_requested() -> bool {
    env_nonempty("RODER_MODELS_REFRESH")
        .or_else(|| env_nonempty("RODER_OPENCODE_MODELS_REFRESH"))
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn responses_tool_name(tool_name: &str, used_tool_names: &mut HashSet<String>) -> String {
    let base_name = responses_api_tool_name(tool_name);
    if used_tool_names.insert(base_name.clone()) {
        return base_name;
    }

    for suffix in 2u32.. {
        let candidate = format!("{base_name}_{suffix}");
        if used_tool_names.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

fn responses_api_tool_name(tool_name: &str) -> String {
    let name = tool_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    if name.is_empty() {
        "tool".to_string()
    } else {
        name
    }
}

fn map_tool_name<'a>(tool_name: &'a str, tool_name_map: &'a HashMap<String, String>) -> &'a str {
    tool_name_map
        .get(tool_name)
        .map_or(tool_name, String::as_str)
}

#[async_trait::async_trait]
impl InferenceEngine for OpenAiResponsesEngine {
    fn id(&self) -> InferenceEngineId {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: true,
            structured_output: true,
            image_input: true,
            prompt_cache: true,
            provider_metadata: true,
            tool_search: true,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        match self.provider_id.as_str() {
            PROVIDER_XAI => InferenceProviderMetadata {
                name: "xAI".to_string(),
                description: Some("xAI API key provider for Grok models".to_string()),
                auth_type: ProviderAuthType::ApiKey,
                auth_label: Some("XAI_API_KEY".to_string()),
                auth_configured: Some(self.api_key.is_some()),
                recommended: false,
                sort_order: 50,
            },
            PROVIDER_OPENROUTER => InferenceProviderMetadata {
                name: "OpenRouter".to_string(),
                description: Some("OpenRouter API key provider for routed models".to_string()),
                auth_type: ProviderAuthType::ApiKey,
                auth_label: Some("OPENROUTER_API_KEY".to_string()),
                auth_configured: Some(self.api_key.is_some()),
                recommended: true,
                sort_order: 18,
            },
            PROVIDER_FIREWORKS => InferenceProviderMetadata {
                name: "Fireworks AI".to_string(),
                description: Some(
                    "Fireworks AI API key provider for account-scoped models".to_string(),
                ),
                auth_type: ProviderAuthType::ApiKey,
                auth_label: Some("FIREWORKS_API_KEY".to_string()),
                auth_configured: Some(self.api_key.is_some()),
                recommended: true,
                sort_order: 19,
            },
            _ => InferenceProviderMetadata {
                name: self.display_name.clone(),
                description: Some(format!("{} OpenAI-compatible provider", self.display_name)),
                auth_type: ProviderAuthType::ApiKey,
                auth_label: Some("API key".to_string()),
                auth_configured: Some(self.api_key.is_some()),
                recommended: true,
                sort_order: 20,
            },
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        let cached = cached_models(&self.provider_id, &self.base_url).ok();
        if self.discover_models {
            let should_refresh = force_refresh_requested()
                || cached
                    .as_ref()
                    .map(|entry| entry.is_stale(cache_ttl()))
                    .unwrap_or(true);
            if should_refresh {
                self.schedule_model_refresh();
            }
        }
        if let Some(entry) = cached
            && !entry.models.is_empty()
        {
            return Ok(entry.models);
        }
        Ok(models_for_provider(&self.provider_id, false))
    }

    async fn compact_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<Option<InferenceEventStream>> {
        native_compaction::compact(self, ctx, request).await
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let Some(api_key) = self.api_key.as_ref() else {
            // Known API-key providers name their env var; custom providers
            // have no canonical one so the guidance stays generic.
            let env_var = match self.provider_id.as_str() {
                PROVIDER_OPENAI => Some("OPENAI_API_KEY"),
                PROVIDER_XAI => Some("XAI_API_KEY"),
                PROVIDER_OPENROUTER => Some("OPENROUTER_API_KEY"),
                PROVIDER_FIREWORKS => Some("FIREWORKS_API_KEY"),
                _ => None,
            };
            match env_var {
                Some(env_var) => anyhow::bail!(
                    "{} API key is missing; set {env_var} or configure it from the provider menu",
                    self.display_name
                ),
                None => anyhow::bail!(
                    "{} API key is missing; configure it from the provider menu or user config",
                    self.display_name
                ),
            }
        };
        let (mut body, tool_name_map) = Self::map_request_with_options(
            &request,
            RequestMappingOptions {
                profile: self.profile,
                thread_id: Some(_ctx.thread_id),
            },
        );
        if self.provider_id == PROVIDER_CODEX {
            body.as_object_mut().unwrap().remove("context_management");
        }
        // Provider-native tool search may require client-executed searches
        // against the runtime catalog within the same turn.
        let continuation =
            openai_provider_native_tool_search(&request).then(|| ClientToolSearchContext {
                base_url: self.base_url.clone(),
                api_key: api_key.clone(),
                headers: self.headers.clone(),
                grok_conversation_id: (self.profile == ResponsesProviderProfile::Xai)
                    .then(|| _ctx.thread_id.to_string()),
                body: body.clone(),
                policy: request.runtime.reliability.clone(),
                definitions: client_search_definitions(&body, &tool_name_map),
                catalog: roder_api::tool_search_catalog::ToolSearchCatalog::build(
                    &request.tools,
                    &request.runtime.tool_search,
                ),
            });
        // Auto-compaction uses the HTTP context_management contract as well.
        if self.profile == ResponsesProviderProfile::OpenAi
            && body.get("context_management").is_none()
            && websocket_requested(&self.base_url)
            && let Some(stream) = try_websocket_stream(
                &self.base_url,
                api_key,
                &self.headers,
                _ctx.thread_id,
                &body,
                tool_name_map.api_name_to_tool_name.clone(),
                continuation.clone(),
            )
            .await?
        {
            return Ok(stream);
        }
        let response = send_responses_request(
            &self.base_url,
            api_key,
            &self.headers,
            (self.profile == ResponsesProviderProfile::Xai).then_some(_ctx.thread_id),
            &body,
            request.runtime.reliability.as_ref(),
        )
        .await
        .map_err(|err| match self.profile {
            ResponsesProviderProfile::Xai => err.context("xAI Responses error"),
            ResponsesProviderProfile::OpenRouter => {
                let message = openrouter_error_message(&err.to_string());
                err.context(message)
            }
            ResponsesProviderProfile::Fireworks => {
                let message = fireworks_error_message(&err.to_string());
                err.context(message)
            }
            ResponsesProviderProfile::OpenAi => err,
        })?;
        Ok(stream_responses_sse_with_client_tool_search(
            response.response,
            tool_name_map.api_name_to_tool_name,
            response.retry_events,
            response.idle_timeout,
            continuation,
        ))
    }
}

fn responses_stream_idle_timeout() -> Duration {
    std::env::var(RESPONSES_STREAM_IDLE_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_RESPONSES_STREAM_IDLE_TIMEOUT)
}

fn openrouter_error_message(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    let detail = error_body_excerpt(error);
    if lower.contains("401") || lower.contains("unauthorized") {
        return format!(
            "OpenRouter auth failed: {detail}. Check OPENROUTER_API_KEY or configure the provider API key."
        );
    }
    if lower.contains("402")
        || lower.contains("insufficient")
        || lower.contains("credit")
        || lower.contains("balance")
    {
        return format!("OpenRouter credits or quota check failed: {detail}.");
    }
    if lower.contains("429") || lower.contains("rate limit") {
        return format!("OpenRouter rate limit reached: {detail}.");
    }
    if lower.contains("context") && (lower.contains("length") || lower.contains("limit")) {
        return format!("OpenRouter context length exceeded: {detail}.");
    }
    if lower.contains("unsupported")
        || lower.contains("invalid parameter")
        || lower.contains("invalid_request")
        || lower.contains("400")
    {
        return format!("OpenRouter rejected a request parameter: {detail}.");
    }
    if (lower.contains("404") || lower.contains("not found") || lower.contains("unavailable"))
        && lower.contains("model")
    {
        return format!("OpenRouter model unavailable: {detail}.");
    }
    if lower.contains("upstream")
        || lower.contains("temporarily unavailable")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
    {
        return format!("OpenRouter upstream provider unavailable: {detail}.");
    }
    format!("OpenRouter Responses error: {detail}")
}

fn fireworks_error_message(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    let detail = error_body_excerpt(error);
    if lower.contains("401") || lower.contains("unauthorized") || lower.contains("403") {
        return format!(
            "Fireworks auth failed: {detail}. Check FIREWORKS_API_KEY or configure the provider API key."
        );
    }
    if lower.contains("402") || lower.contains("payment") || lower.contains("billing") {
        return format!("Fireworks billing or quota check failed: {detail}.");
    }
    if lower.contains("404") && lower.contains("model")
        || lower.contains("not found") && lower.contains("model")
        || lower.contains("not deployed")
    {
        return format!("Fireworks model unavailable: {detail}.");
    }
    if lower.contains("413") || lower.contains("payload too large") {
        return format!("Fireworks payload too large: {detail}.");
    }
    if lower.contains("408") || lower.contains("timeout") {
        return format!("Fireworks request timed out: {detail}.");
    }
    if lower.contains("429") || lower.contains("rate limit") || lower.contains("capacity") {
        return format!("Fireworks rate limit or capacity reached: {detail}.");
    }
    if lower.contains("400") || lower.contains("invalid") || lower.contains("malformed") {
        return format!("Fireworks rejected a request parameter: {detail}.");
    }
    if lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("520")
        || lower.contains("unavailable")
    {
        return format!("Fireworks upstream service unavailable: {detail}.");
    }
    format!("Fireworks Responses error: {detail}")
}

fn push_retry_event(
    events: &mut Vec<Value>,
    attempt: u32,
    cause: &str,
    policy: &ReliabilityRequestPolicy,
) {
    events.push(provider_retry_metadata(attempt, cause, policy));
}

async fn retry_sleep(policy: &ReliabilityRequestPolicy, attempt: u32) {
    let delay = provider_retry_delay_ms(policy, attempt);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
}

#[cfg(test)]
fn xai_error_message(status: reqwest::StatusCode, body: &str) -> String {
    let trimmed = body.trim();
    let detail = if trimmed.is_empty() {
        "empty response body"
    } else {
        trimmed
    };
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return format!(
            "xAI auth failed ({status}): {detail}. Check XAI_API_KEY or run `roder auth login supergrok` for SuperGrok."
        );
    }
    if status == reqwest::StatusCode::FORBIDDEN {
        return format!(
            "xAI entitlement or quota check failed ({status}): {detail}. X Premium+ may not include the required SuperGrok/API entitlement; verify Grok usage and subscription settings or switch providers."
        );
    }
    format!("xAI Responses error {status}: {detail}")
}

fn stream_error_message(data: &Value, fallback: &str) -> String {
    data.get("response")
        .and_then(|response| response.get("error"))
        .or_else(|| data.get("error"))
        .and_then(|error| {
            error
                .get("message")
                .and_then(|message| message.as_str())
                .or_else(|| error.as_str())
        })
        .or_else(|| data.get("message").and_then(|message| message.as_str()))
        .unwrap_or(fallback)
        .to_string()
}

fn error_body_excerpt(body: &str) -> String {
    const MAX_ERROR_BODY_CHARS: usize = 2_000;
    let mut excerpt = body.chars().take(MAX_ERROR_BODY_CHARS).collect::<String>();
    if body.chars().count() > MAX_ERROR_BODY_CHARS {
        excerpt.push_str(" ...");
    }
    excerpt
}

fn is_compaction_item(item: &Value) -> bool {
    item.get("type")
        .and_then(Value::as_str)
        .is_some_and(is_compaction_type)
}

fn is_compaction_type(kind: &str) -> bool {
    kind.contains("compaction")
}

fn compaction_event(item: &Value, status: &str) -> InferenceEvent {
    InferenceEvent::Compaction(CompactionProgress {
        status: status.to_string(),
        item_id: item.get("id").and_then(Value::as_str).map(str::to_string),
        tokens_before: None,
        tokens_after: None,
        duration_ms: None,
        // Always attach the opaque item so the runtime can persist a boundary
        // even when the SSE stream aborts before response.completed.
        item: Some(item.clone()),
    })
}

#[cfg(test)]
fn extract_response_text(value: &Value) -> String {
    value
        .get("output_text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| extract_output_text(value))
        .unwrap_or_default()
}

fn message_deltas_from_response(
    value: &Value,
    state: &mut ResponsesStreamState,
) -> Vec<InferenceEvent> {
    let Some(output) = value.get("output").and_then(Value::as_array) else {
        let text = value
            .get("output_text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if state.streamed_final_text || text.is_empty() {
            return Vec::new();
        }
        state.streamed_final_text = true;
        return vec![InferenceEvent::MessageDelta(MessageDelta {
            text: text.to_string(),
            phase: Some(FINAL_ANSWER_PHASE.to_string()),
        })];
    };

    output
        .iter()
        .filter_map(|item| message_delta_from_done_item(item, state))
        .collect()
}

fn extract_tool_calls(
    value: &Value,
    tool_name_map: &HashMap<String, String>,
) -> Vec<ToolCallCompleted> {
    let Some(output) = value.get("output").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    output
        .iter()
        .flat_map(|item| extract_tool_calls_from_item(item, tool_name_map))
        .collect()
}

fn extract_hosted_tool_calls(value: &Value) -> Vec<HostedToolCallCompleted> {
    let Some(output) = value.get("output").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    output
        .iter()
        // Client-executed searches complete only after the local search.
        .filter(|item| !is_client_executed_tool_search(item))
        .filter_map(hosted_tool_call_completed_from_item)
        .collect()
}

#[cfg(test)]
fn extract_output_text(value: &Value) -> Option<String> {
    let output = value.get("output")?.as_array()?;
    let mut parts = Vec::new();
    for item in output {
        let is_final_answer = item
            .get("phase")
            .and_then(|v| v.as_str())
            .is_none_or(|phase| phase == "final_answer");
        if !is_final_answer {
            continue;
        }
        if let Some(content) = item.get("content") {
            if let Some(text) = content.as_str() {
                parts.push(text.to_string());
                continue;
            }
            if let Some(blocks) = content.as_array() {
                for block in blocks {
                    if let Some(text) = block
                        .get("text")
                        .or_else(|| block.get("output_text"))
                        .and_then(|v| v.as_str())
                    {
                        parts.push(text.to_string());
                    }
                }
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join(""))
}

fn output_text_from_message_item(item: &Value) -> Option<String> {
    let content = item.get("content")?;
    if let Some(text) = content.as_str() {
        return (!text.is_empty()).then(|| text.to_string());
    }
    let content = content.as_array()?;
    let mut parts = Vec::new();
    for block in content {
        if let Some(text) = block
            .get("text")
            .or_else(|| block.get("output_text"))
            .and_then(Value::as_str)
        {
            parts.push(text.to_string());
        }
    }
    (!parts.is_empty()).then(|| parts.join(""))
}

fn extract_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let prompt_tokens = number_to_u32(usage.get("input_tokens"));
    let completion_tokens = number_to_u32(usage.get("output_tokens"));
    let total_tokens = number_to_u32(usage.get("total_tokens"))
        .or_else(|| Some(prompt_tokens? + completion_tokens?));
    let cached_prompt_tokens =
        number_to_u32(usage.pointer("/input_tokens_details/cached_tokens")).unwrap_or_default();
    Some(
        TokenUsage::new(
            prompt_tokens.unwrap_or_default(),
            completion_tokens.unwrap_or_default(),
            total_tokens.unwrap_or_default(),
        )
        .with_cached_prompt_tokens(cached_prompt_tokens)
        .with_service_tier(
            value
                .get("service_tier")
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
    )
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
#[path = "tool_search_stream_tests.rs"]
mod tool_search_stream_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };
    use roder_api::reliability::ReliabilityRequestPolicy;
    use roder_api::transcript::{
        AssistantMessage, InputImage, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    pub(super) fn request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: "openai".to_string(),
                model: "gpt-5.5".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("be helpful".to_string()),
                developer: None,
                developer_context: None,
            },
            transcript: vec![
                TranscriptItem::UserMessage(UserMessage::text("Hello")),
                TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "Hi".to_string(),
                    phase: None,
                }),
            ],
            tools: vec![roder_api::tools::ToolSpec {
                name: "echo".to_string(),
                description: "echo text".to_string(),
                parameters: json!({ "type": "object" }),
            }],
            tool_choice: roder_api::tools::ToolChoice::Auto,
            reasoning: ReasoningConfig {
                enabled: true,
                level: Some("medium".to_string()),
            },
            output: OutputConfig {
                max_tokens: Some(200),
                temperature: Some(0.2),
                top_p: None,
                response_format: Some(json!({ "type": "json_object" })),
            },
            runtime: RuntimeHints {
                trace_id: None,
                prompt_cache_key: Some("cache-key".to_string()),
                auto_compact_token_limit: Some(200_000),
                parallel_tool_calls: Some(true),
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                ..RuntimeHints::default()
            },
            metadata: json!({}),
        }
    }

    fn input_items(request: &AgentInferenceRequest) -> Vec<Value> {
        let (_, tool_name_map) = responses_tools(request, ResponsesProviderProfile::OpenAi);
        response_input_items(
            request,
            &tool_name_map,
            ResponsesProviderProfile::OpenAi,
            true,
        )
    }

    #[test]
    fn maps_openai_provider_native_tool_search_body() {
        let mut request = request();
        request.model.model = "gpt-5.4".to_string();
        request.runtime.tool_search = roder_api::inference::ToolSearchConfig::provider_native();

        let body = OpenAiResponsesEngine::map_request(&request);

        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "echo");
        assert_eq!(body["tools"][0]["defer_loading"], true);
        assert_eq!(body["tools"][1]["type"], "tool_search");
        assert_eq!(body["tools"][1]["execution"], "client");
        assert_eq!(body["tools"][1]["parameters"]["required"], json!(["query"]));
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn gpt_6_supports_provider_native_tool_search() {
        for model in ["gpt-6-astra", "gpt-6-sol", "gpt-6-luna"] {
            assert!(openai_model_supports_tool_search(model));
        }
    }

    #[test]
    fn keeps_explicit_openai_tools_for_unsupported_tool_search_model() {
        let mut request = request();
        request.model.model = "gpt-5.3".to_string();
        request.runtime.tool_search = roder_api::inference::ToolSearchConfig::provider_native();

        let body = OpenAiResponsesEngine::map_request(&request);

        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert!(body["tools"][0].get("defer_loading").is_none());
    }

    #[test]
    fn maps_fireworks_request_without_openai_only_state_fields() {
        let mut request = request();
        request.model.provider = PROVIDER_FIREWORKS.to_string();
        request.model.model = "accounts/fireworks/models/qwen3-235b-a22b".to_string();

        let (body, _) = OpenAiResponsesEngine::map_request_with_options(
            &request,
            RequestMappingOptions {
                profile: ResponsesProviderProfile::Fireworks,
                thread_id: Some("thread-a"),
            },
        );

        assert_eq!(body["model"], "accounts/fireworks/models/qwen3-235b-a22b");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tool_choice"], "auto");
        assert!(body.get("instructions").is_none());
        assert_eq!(body["input"][0]["role"], "system");
        assert_eq!(body["input"][0]["content"][0]["text"], "be helpful");
        assert!(body.get("include").is_none());
        assert!(body.get("reasoning").is_none());
        assert!(body.get("prompt_cache_key").is_none());
        assert!(body.get("context_management").is_none());
        assert!(body.get("previous_response_id").is_none());
    }

    #[tokio::test]
    async fn model_discovery_reads_models_endpoint() {
        let base_url = spawn_models_server(vec![(
            "/models",
            200,
            r#"{"data":[{"id":"custom-alpha","name":"Custom Alpha"}]}"#,
        )])
        .await;

        let models = discover_models(&base_url, Some("secret")).await.unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "custom-alpha");
        assert_eq!(models[0].name, "Custom Alpha");
    }

    #[tokio::test]
    async fn model_discovery_attaches_catalog_max_thinking_for_known_ids() {
        let base_url = spawn_models_server(vec![(
            "/models",
            200,
            r#"{"data":[{"id":"gpt-5.6-sol","name":"Sol From API"}]}"#,
        )])
        .await;

        let models = discover_models(&base_url, Some("secret")).await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-5.6-sol");
        let efforts: Vec<_> = models[0]
            .supported_reasoning
            .iter()
            .map(|effort| effort.effort.as_str())
            .collect();
        assert!(
            efforts.contains(&"max"),
            "known Sol discovery should expose max thinking; got {efforts:?}"
        );
        assert!(
            efforts.contains(&"ultra"),
            "known Sol discovery should expose ultra; got {efforts:?}"
        );
    }

    #[tokio::test]
    async fn model_discovery_falls_back_to_v1_models_endpoint() {
        let base_url = spawn_models_server(vec![
            ("/models", 404, r#"{"error":"missing"}"#),
            (
                "/v1/models",
                200,
                r#"{"data":[{"id":"custom-v1","name":"Custom V1"}]}"#,
            ),
        ])
        .await;

        let models = discover_models(&base_url, None).await.unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "custom-v1");
    }

    #[tokio::test]
    async fn model_discovery_preserves_openrouter_slash_ids_and_context_length() {
        let base_url = spawn_models_server(vec![(
            "/models",
            200,
            r#"{"data":[{"id":"x-ai/grok-4.6","name":"Grok 4.6","context_length":500000}]}"#,
        )])
        .await;

        let models = discover_models(&base_url, Some("secret")).await.unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "x-ai/grok-4.6");
        assert_eq!(models[0].name, "Grok 4.6");
        assert_eq!(models[0].context_window, Some(500_000));
    }

    #[tokio::test]
    async fn model_discovery_preserves_fireworks_account_scoped_ids() {
        let base_url = spawn_models_server(vec![(
            "/models",
            200,
            r#"{"data":[{"id":"accounts/fireworks/models/qwen3-235b-a22b","name":"Qwen3 235B A22B","context_length":131072}]}"#,
        )])
        .await;

        let models = discover_models(&base_url, Some("secret")).await.unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "accounts/fireworks/models/qwen3-235b-a22b");
        assert_eq!(models[0].name, "Qwen3 235B A22B");
        assert_eq!(models[0].context_window, Some(131_072));
    }

    #[tokio::test]
    async fn retry_recovers_responses_request_after_retryable_status() {
        let base_url = spawn_models_server(vec![
            ("/responses", 429, r#"{"error":"busy"}"#),
            (
                "/responses",
                200,
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            ),
        ])
        .await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_max_attempts: 2,
            provider_retry_initial_backoff_ms: 0,
            provider_retry_status_codes: vec![429],
            ..ReliabilityRequestPolicy::default()
        };

        let response = send_responses_request(
            &base_url,
            "secret",
            &[],
            None,
            &json!({ "model": "gpt-5.5" }),
            Some(&policy),
        )
        .await
        .unwrap();

        assert!(response.response.status().is_success());
        assert_eq!(
            response.retry_events[0]["kind"],
            "reliability_retry_attempt"
        );
    }

    #[tokio::test]
    async fn client_executed_tool_search_continues_the_turn_with_search_output() {
        use roder_api::inference::ToolSearchConfig;
        use roder_api::tool_search_catalog::ToolSearchCatalog;

        const FIRST_SSE: &str = "data: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"ts_9\",\"type\":\"tool_search_call\",\"status\":\"completed\",\"call_id\":\"search_9\",\"execution\":\"client\",\"arguments\":{\"query\":\"read files\",\"limit\":1}}}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"output\":[{\"id\":\"ts_9\",\"type\":\"tool_search_call\",\"status\":\"completed\",\"call_id\":\"search_9\",\"execution\":\"client\",\"arguments\":{\"query\":\"read files\",\"limit\":1}}]}}\n\n";
        const SECOND_SSE: &str = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_2\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"done\"}]}]}}\n\n";

        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let base_url = spawn_recording_server(
            vec![
                ("/responses", 200, FIRST_SSE),
                ("/responses", 200, SECOND_SSE),
            ],
            Arc::clone(&bodies),
        )
        .await;

        let body = json!({
            "model": "gpt-5.5",
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "go" }] }
            ]
        });
        let response = send_responses_request(&base_url, "secret", &[], None, &body, None)
            .await
            .unwrap();

        let tools = vec![
            roder_api::tools::ToolSpec {
                name: "read_file".to_string(),
                description: "Read a file from the workspace".to_string(),
                parameters: json!({ "type": "object" }),
            },
            roder_api::tools::ToolSpec {
                name: "deploy_app".to_string(),
                description: "Deploy the application".to_string(),
                parameters: json!({ "type": "object" }),
            },
        ];
        let catalog = ToolSearchCatalog::build(&tools, &ToolSearchConfig::default());
        let mut stream = stream_responses_sse_with_client_tool_search(
            response.response,
            HashMap::new(),
            response.retry_events,
            response.idle_timeout,
            Some(ClientToolSearchContext {
                base_url: base_url.clone(),
                api_key: "secret".to_string(),
                headers: Vec::new(),
                grok_conversation_id: None,
                body: body.clone(),
                policy: None,
                definitions: tools.iter().map(|tool| (tool.name.clone(), json!({ "type": "function", "name": tool.name, "description": tool.description, "parameters": tool.parameters }))).collect(),
                catalog,
            }),
        );

        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.expect("stream event"));
        }

        // Exactly one turn completion, from the continuation response.
        let completions: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                InferenceEvent::Completed(metadata) => Some(metadata),
                _ => None,
            })
            .collect();
        assert_eq!(completions.len(), 1, "{events:?}");
        assert_eq!(
            completions[0].provider_response_id.as_deref(),
            Some("resp_2")
        );

        // The locally-executed search surfaces through the canonical hosted
        // lifecycle with searched tool ids preserved.
        let search_completed = events
            .iter()
            .find_map(|event| match event {
                InferenceEvent::HostedToolCallCompleted(call) if call.name == "tool_search" => {
                    Some(call)
                }
                _ => None,
            })
            .expect("client search completion");
        let arguments: Value = serde_json::from_str(&search_completed.arguments).unwrap();
        assert_eq!(arguments["executor"], "client");
        assert_eq!(arguments["selected_tools"][0], "read_file");

        // The continuation request echoed the call and carried the
        // tool_search_output with catalog payloads.
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2);
        let second: Value = serde_json::from_str(&bodies[1]).unwrap();
        let input = second["input"].as_array().unwrap();
        let echoed = input
            .iter()
            .find(|item| item["type"] == "tool_search_call")
            .expect("echoed tool_search_call");
        assert_eq!(echoed["id"], "ts_9");
        let output = input
            .iter()
            .find(|item| item["type"] == "tool_search_output")
            .expect("tool_search_output item");
        assert_eq!(output["call_id"], "search_9");
        assert_eq!(output["status"], "completed");
        assert_eq!(output["execution"], "client");
        assert_eq!(output["tools"].as_array().unwrap().len(), 1);
        assert_eq!(output["tools"][0]["name"], "read_file");
        assert_eq!(output["tools"][0]["parameters"], json!({"type": "object"}));
    }

    #[tokio::test]
    async fn invalid_history_fails_without_discarding_tool_results() {
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let base_url = spawn_recording_server(
            vec![
                (
                    "/responses",
                    400,
                    r#"{"error":{"message":"No tool call found for function call output with call_id call_missing.","type":"invalid_request_error","param":"input","code":null}}"#,
                ),
                (
                    "/responses",
                    200,
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
                ),
            ],
            Arc::clone(&bodies),
        )
        .await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_max_attempts: 2,
            provider_retry_initial_backoff_ms: 0,
            ..ReliabilityRequestPolicy::default()
        };

        let error = send_responses_request(
            &base_url,
            "secret",
            &[],
            None,
            &json!({
                "model": "gpt-5.5",
                "input": [
                    { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "continue" }] },
                    { "type": "function_call_output", "call_id": "call_missing", "output": "stale" },
                    { "type": "function_call_output", "call_id": "call_ok", "output": "keep" }
                ]
            }),
            Some(&policy),
        )
        .await
        .err().unwrap();

        assert_eq!(
            error.downcast_ref::<ProviderFailure>().unwrap().kind,
            ProviderFailureKind::InvalidRequest
        );
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains("call_missing"));
        assert!(bodies[0].contains("call_ok"));
    }

    #[tokio::test]
    async fn responses_stream_surfaces_silent_provider_timeout() {
        let base_url = spawn_silent_responses_server().await;
        let response = send_responses_request_with_idle_timeout(
            &base_url,
            "secret",
            &[],
            None,
            &json!({ "model": "gpt-5.5" }),
            None,
            Duration::from_millis(50),
        )
        .await
        .unwrap();

        let mut stream = stream_responses_sse_with_client_tool_search(
            response.response,
            HashMap::new(),
            Vec::new(),
            response.idle_timeout,
            None,
        );
        let next = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("silent responses stream should surface an idle timeout");

        let error = next
            .expect("stream should yield the timeout error")
            .expect_err("silent responses stream should yield an error");

        assert_eq!(
            error.to_string(),
            "Responses stream read failed: kind=timeout; idle_timeout_ms=50; provider_request_id=req_timeout_test; cause=error decoding response body: request or response body error: operation timed out"
        );
    }

    #[tokio::test]
    async fn responses_request_surfaces_silent_headers_timeout() {
        let base_url = spawn_silent_headers_server().await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_max_attempts: 1,
            ..ReliabilityRequestPolicy::default()
        };
        let result = send_responses_request_with_idle_timeout(
            &base_url,
            "secret",
            &[],
            None,
            &json!({ "model": "gpt-5.5" }),
            Some(&policy),
            Duration::from_millis(50),
        )
        .await;
        let error = match result {
            Ok(_) => panic!("silent response headers should time out"),
            Err(error) => error,
        };

        assert!(
            error.to_string().contains("timed out") || error.to_string().contains("timeout"),
            "expected timeout error, got {error:#}"
        );
    }

    #[tokio::test]
    async fn custom_provider_list_models_returns_cached_models_without_waiting_for_refresh() {
        let cache_file = std::env::temp_dir().join(format!(
            "roder-custom-models-cache-{}-{}.json",
            std::process::id(),
            now_unix_secs()
        ));
        let cached_model = ModelDescriptor {
            id: "cached-custom-model".to_string(),
            name: "Cached Custom Model".to_string(),
            context_window: None,
            default_reasoning: None,
            supported_reasoning: Vec::new(),
        };
        unsafe {
            std::env::set_var("RODER_MODELS_CACHE_PATH", &cache_file);
            std::env::set_var("RODER_MODELS_CACHE_TTL_SECONDS", "0");
        }
        save_cached_models(
            "custom-provider",
            "http://127.0.0.1:9",
            std::slice::from_ref(&cached_model),
        )
        .unwrap();
        let engine = OpenAiResponsesEngine::new_custom_provider(
            Some("secret".to_string()),
            "custom-provider",
            "Custom Provider",
            "http://127.0.0.1:9",
        );

        let models = tokio::time::timeout(
            Duration::from_millis(100),
            engine.list_models(InferenceProviderContext {
                provider_id: "custom-provider",
            }),
        )
        .await
        .expect("cached custom model listing should not wait for background refresh")
        .unwrap();

        assert_eq!(models, vec![cached_model]);
        unsafe {
            std::env::remove_var("RODER_MODELS_CACHE_PATH");
            std::env::remove_var("RODER_MODELS_CACHE_TTL_SECONDS");
        }
        let _ = fs::remove_file(cache_file);
    }

    #[tokio::test]
    async fn stream_turn_without_key_fails_naming_the_env_var() {
        let engine = OpenAiResponsesEngine::new(None);
        assert_eq!(engine.metadata().auth_configured, Some(false));

        let err = match engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                request(),
            )
            .await
        {
            Ok(_) => panic!("missing key must fail at call time"),
            Err(err) => err,
        };

        assert_eq!(
            err.to_string(),
            "OpenAI API key is missing; set OPENAI_API_KEY or configure it from the provider menu"
        );
    }

    #[tokio::test]
    async fn xai_stream_turn_without_key_fails_naming_the_env_var() {
        let engine = OpenAiResponsesEngine::new_with_config(
            None,
            PROVIDER_XAI,
            "http://127.0.0.1:9",
            Vec::new(),
        );

        let err = match engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                request(),
            )
            .await
        {
            Ok(_) => panic!("missing key must fail at call time"),
            Err(err) => err,
        };

        assert_eq!(
            err.to_string(),
            "xAI API key is missing; set XAI_API_KEY or configure it from the provider menu"
        );
    }

    async fn spawn_models_server(routes: Vec<(&'static str, u16, &'static str)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (expected_path, status, body) in routes {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = [0_u8; 2048];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("");
                assert_eq!(path, expected_path);
                let status_text = if status == 200 { "OK" } else { "Not Found" };
                let response = format!(
                    "HTTP/1.1 {status} {status_text}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        format!("http://{addr}")
    }

    async fn spawn_recording_server(
        routes: Vec<(&'static str, u16, &'static str)>,
        bodies: Arc<std::sync::Mutex<Vec<String>>>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (expected_path, status, body) in routes {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = [0_u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("");
                assert_eq!(path, expected_path);
                bodies
                    .lock()
                    .unwrap()
                    .push(http_request_body(&request).to_string());
                let status_text = if status == 200 { "OK" } else { "Bad Request" };
                let response = format!(
                    "HTTP/1.1 {status} {status_text}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        format!("http://{addr}")
    }

    fn http_request_body(request: &str) -> &str {
        request
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .unwrap_or("")
    }

    async fn spawn_silent_responses_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = [0_u8; 2048];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("");
            assert_eq!(path, "/responses");
            let response = concat!(
                "HTTP/1.1 200 OK\r\n",
                "content-type: text/event-stream\r\n",
                "x-request-id: req_timeout_test\r\n",
                "connection: keep-alive\r\n",
                "\r\n"
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        format!("http://{addr}")
    }

    async fn spawn_silent_headers_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = [0_u8; 2048];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("");
            assert_eq!(path, "/responses");
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        format!("http://{addr}")
    }

    #[test]
    fn maps_responses_request_options_and_input_items() {
        let body = OpenAiResponsesEngine::map_request(&request());
        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["instructions"], "be helpful");
        assert_eq!(body["max_output_tokens"], 200);
        assert!((body["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["reasoning"]["summary"], "auto");
        assert_eq!(body["include"][0], "reasoning.encrypted_content");
        assert_eq!(body["prompt_cache_key"], "cache-key");
        assert_eq!(
            body["context_management"][0],
            json!({ "type": "compaction", "compact_threshold": 200_000 })
        );
        assert_eq!(body["text"]["format"]["type"], "json_object");
        assert_eq!(body["tools"][0]["name"], "echo");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["parallel_tool_calls"], true);
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][1]["role"], "assistant");
        assert_eq!(body["input"][1]["phase"], "final_answer");
    }

    #[test]
    fn maps_developer_and_ultra_policy_into_openai_instructions() {
        let mut request = request();
        request.instructions.developer = Some(
            "Proactive multi-agent delegation is active.\n\nAgent-control workflow:\n- spawn_agent"
                .to_string(),
        );
        request.instructions.developer_context =
            Some("Per-turn identity for /root/lead".to_string());

        let body = OpenAiResponsesEngine::map_request(&request);
        let instructions = body["instructions"].as_str().expect("instructions");
        assert!(instructions.contains("be helpful"));
        assert!(instructions.contains("Developer instructions:"));
        assert!(instructions.contains("Proactive multi-agent delegation is active"));
        assert!(instructions.contains("spawn_agent"));
        assert!(
            !instructions.contains("Per-turn identity"),
            "developer_context must stay out of the stable instructions prefix"
        );
        assert_eq!(body["input"][0]["role"], "system");
        assert_eq!(
            body["input"][0]["content"][0]["text"],
            "Developer context (this turn):\nPer-turn identity for /root/lead"
        );
        assert_eq!(body["input"][1]["role"], "user");
    }

    #[test]
    fn xai_and_supergrok_mapping_includes_developer_ultra_policy() {
        let mut request = request();
        request.model.provider = PROVIDER_SUPERGROK.to_string();
        request.model.model = "grok-4.6".to_string();
        request.instructions.developer = Some(
            "Proactive multi-agent delegation is active. Prefer spawn_agent for parallel work."
                .to_string(),
        );
        request.instructions.developer_context = Some("child path /root/probe".to_string());

        let (body, _) = OpenAiResponsesEngine::map_request_with_options(
            &request,
            RequestMappingOptions {
                profile: ResponsesProviderProfile::Xai,
                thread_id: Some("thread-ultra"),
            },
        );

        let instructions = body["instructions"].as_str().expect("xai instructions");
        assert!(instructions.contains("be helpful"));
        assert!(instructions.contains("Developer instructions:"));
        assert!(instructions.contains("Proactive multi-agent delegation is active"));
        assert!(instructions.contains("spawn_agent"));
        assert!(!instructions.contains("child path /root/probe"));
        assert_eq!(
            body["input"][0]["content"][0]["text"],
            "Developer context (this turn):\nchild path /root/probe"
        );
    }

    #[test]
    fn maps_parallel_tool_call_override_for_responses_requests() {
        let mut request = request();
        request.runtime.parallel_tool_calls = Some(false);

        let body = OpenAiResponsesEngine::map_request(&request);

        assert_eq!(body["parallel_tool_calls"], false);
    }

    #[test]
    fn maps_ultra_mode_to_max_openai_reasoning_effort() {
        let mut request = request();
        request.reasoning.level = Some(REASONING_ULTRA.to_string());

        let body = OpenAiResponsesEngine::map_request(&request);

        assert_eq!(body["reasoning"]["effort"], REASONING_MAX);
        assert_eq!(request.reasoning.level.as_deref(), Some(REASONING_ULTRA));
    }

    #[test]
    fn profile_request_snapshot_maps_openai_patch_reasoning_parallel_and_context() {
        let mut request = request();
        request.tools = vec![roder_api::tools::ToolSpec {
            name: "apply_patch".to_string(),
            description: "Apply a patch".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["patch"],
                "properties": { "patch": { "type": "string" } },
                "additionalProperties": false
            }),
        }];
        request.reasoning.level = Some("high".to_string());
        request.runtime.parallel_tool_calls = Some(false);
        request.runtime.auto_compact_token_limit = Some(180_000);
        request.metadata = json!({
            "modelProfile": {
                "editTool": "patch",
                "schemaPolicy": "required_first_flat",
                "parallelToolCalls": false
            }
        });

        let body = OpenAiResponsesEngine::map_request(&request);

        assert_eq!(body["tools"][0]["name"], "apply_patch");
        // gpt-5.5 receives apply_patch on the freeform/custom channel.
        assert_eq!(body["tools"][0]["type"], "custom");
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["parallel_tool_calls"], false);
        assert_eq!(
            body["context_management"][0],
            json!({ "type": "compaction", "compact_threshold": 180_000 })
        );
    }

    #[test]
    fn preserves_assistant_message_phase_for_responses_replay() {
        let mut request = request();
        request.transcript = vec![TranscriptItem::AssistantMessage(AssistantMessage {
            text: "I will inspect first.".to_string(),
            phase: Some("commentary".to_string()),
        })];

        let input = input_items(&request);
        assert_eq!(input[0]["role"], "assistant");
        assert_eq!(input[0]["phase"], "commentary");
        assert_eq!(input[0]["content"][0]["text"], "I will inspect first.");
    }

    #[test]
    fn maps_user_images_to_responses_input_image_content() {
        let mut request = request();
        request.transcript = vec![TranscriptItem::UserMessage(UserMessage::with_images(
            "what is shown?",
            vec![InputImage {
                image_url: "data:image/png;base64,YWJj".to_string(),
            }],
        ))];

        let input = input_items(&request);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "what is shown?");
        assert_eq!(input[0]["content"][1]["type"], "input_image");
        assert_eq!(
            input[0]["content"][1]["image_url"],
            "data:image/png;base64,YWJj"
        );
    }

    #[test]
    fn xai_mapping_strips_images_for_composer_model() {
        let mut request = request();
        request.model.provider = PROVIDER_SUPERGROK.to_string();
        request.model.model = "grok-composer-2.5-fast".to_string();
        request.transcript = vec![TranscriptItem::UserMessage(UserMessage::with_images(
            "what is shown?",
            vec![InputImage {
                image_url: "data:image/png;base64,YWJj".to_string(),
            }],
        ))];

        let (body, _) = OpenAiResponsesEngine::map_request_with_options(
            &request,
            RequestMappingOptions {
                profile: ResponsesProviderProfile::Xai,
                thread_id: None,
            },
        );
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"].as_array().unwrap().len(), 1);
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "what is shown?");
    }

    #[test]
    fn xai_mapping_keeps_images_for_vision_model() {
        let mut request = request();
        request.model.provider = PROVIDER_SUPERGROK.to_string();
        request.model.model = "grok-4.3".to_string();
        request.transcript = vec![TranscriptItem::UserMessage(UserMessage::with_images(
            "what is shown?",
            vec![InputImage {
                image_url: "data:image/png;base64,YWJj".to_string(),
            }],
        ))];

        let (body, _) = OpenAiResponsesEngine::map_request_with_options(
            &request,
            RequestMappingOptions {
                profile: ResponsesProviderProfile::Xai,
                thread_id: None,
            },
        );
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "what is shown?");
        assert_eq!(input[0]["content"][1]["type"], "input_image");
        assert_eq!(
            input[0]["content"][1]["image_url"],
            "data:image/png;base64,YWJj"
        );
    }

    #[test]
    fn maps_apply_patch_tool_for_responses_requests() {
        let mut request = request();
        request.tools = vec![roder_api::tools::ToolSpec {
            name: "apply_patch".to_string(),
            description: "Apply a patch".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "patch": { "type": "string" } },
                "required": ["patch"],
                "additionalProperties": false
            }),
        }];

        let body = OpenAiResponsesEngine::map_request(&request);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        // gpt-5.5 advertises apply_patch on the freeform/custom channel: a bare
        // `type:"custom"` tool with no JSON parameters schema.
        assert_eq!(tools[0]["type"], "custom");
        assert_eq!(tools[0]["name"], "apply_patch");
        assert!(tools[0].get("parameters").is_none());
    }

    #[test]
    fn keeps_apply_patch_as_function_for_models_without_custom_tools() {
        let mut request = request();
        request.model.model = "gpt-4.1".to_string();
        request.tools = vec![roder_api::tools::ToolSpec {
            name: "apply_patch".to_string(),
            description: "Apply a patch".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "patch": { "type": "string" } },
                "required": ["patch"],
                "additionalProperties": false
            }),
        }];

        let body = OpenAiResponsesEngine::map_request(&request);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "apply_patch");
        assert_eq!(tools[0]["parameters"]["required"][0], "patch");
    }

    #[test]
    fn parses_custom_tool_call_into_dispatch_arguments() {
        let response = json!({
            "output": [
                {
                    "type": "custom_tool_call",
                    "id": "ctc_1",
                    "call_id": "call_patch",
                    "name": "apply_patch",
                    "input": "*** Begin Patch\n*** End Patch\n"
                }
            ]
        });
        let calls = extract_tool_calls(&response, &HashMap::new());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_patch");
        assert_eq!(calls[0].name, "apply_patch");
        assert_eq!(
            calls[0].arguments,
            json!({ "patch": "*** Begin Patch\n*** End Patch\n" }).to_string()
        );
    }

    #[test]
    fn replays_custom_tool_call_output_for_freeform_results() {
        let mut request = request();
        request.transcript = vec![
            TranscriptItem::ProviderMetadata(json!({
                "output": [
                    {
                        "type": "custom_tool_call",
                        "id": "ctc_1",
                        "call_id": "call_patch",
                        "name": "apply_patch",
                        "input": "*** Begin Patch\n*** End Patch\n"
                    }
                ]
            })),
            TranscriptItem::ToolCall(ToolCallRecord {
                id: "call_patch".to_string(),
                name: "apply_patch".to_string(),
                arguments: json!({ "patch": "*** Begin Patch\n*** End Patch\n" }).to_string(),
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "call_patch".to_string(),
                name: Some("apply_patch".to_string()),
                result: "Success.".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];

        let items = input_items(&request);
        let call = items
            .iter()
            .find(|item| item["type"] == "custom_tool_call")
            .expect("custom_tool_call replayed");
        assert_eq!(call["call_id"], "call_patch");
        assert_eq!(call["input"], "*** Begin Patch\n*** End Patch\n");
        let output = items
            .iter()
            .find(|item| item["type"] == "custom_tool_call_output")
            .expect("custom_tool_call_output replayed");
        assert_eq!(output["call_id"], "call_patch");
        assert_eq!(output["output"], "Success.");
    }

    #[test]
    fn normalizes_tool_schema_order_for_responses_tools() {
        let mut request = request();
        request.tools = vec![roder_api::tools::ToolSpec {
            name: "shell".to_string(),
            description: "Run shell command".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "additionalProperties": false,
                "required": ["command"]
            }),
        }];

        let body = OpenAiResponsesEngine::map_request(&request);
        let schema = serde_json::to_string(&body["tools"][0]["parameters"]).unwrap();

        assert!(
            schema.starts_with(r#"{"type":"object","required":["command"],"properties":"#),
            "{schema}"
        );
    }

    #[test]
    fn maps_hosted_web_search_for_responses_requests() {
        let mut request = request();
        request.runtime.hosted_web_search = roder_api::inference::HostedWebSearchConfig::cached();

        let body = OpenAiResponsesEngine::map_request(&request);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["type"], "web_search");
        assert_eq!(tools[0]["external_web_access"], false);
        assert_eq!(tools[1]["type"], "function");
        assert_eq!(tools[1]["name"], "echo");
    }

    #[test]
    fn maps_unsafe_function_tool_names_to_openai_safe_names() {
        let mut request = request();
        request.tools = vec![
            roder_api::tools::ToolSpec {
                name: "memory.save".to_string(),
                description: "save memory entry".to_string(),
                parameters: json!({ "type": "object" }),
            },
            roder_api::tools::ToolSpec {
                name: "memory.query".to_string(),
                description: "query memory".to_string(),
                parameters: json!({ "type": "object" }),
            },
        ];
        request.tool_choice = roder_api::tools::ToolChoice::Specific("memory.save".to_string());

        let body = OpenAiResponsesEngine::map_request(&request);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "memory_save");
        assert_eq!(tools[1]["type"], "function");
        assert_eq!(tools[1]["name"], "memory_query");
        assert_eq!(body["tool_choice"]["name"], "memory_save");
    }

    #[test]
    fn replays_unsafe_tool_call_names_as_openai_safe_names() {
        let mut request = request();
        request.tools = vec![roder_api::tools::ToolSpec {
            name: "tool.discovery.list".to_string(),
            description: "list discovery tools".to_string(),
            parameters: json!({ "type": "object" }),
        }];
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("List discovery tools")),
            TranscriptItem::ToolCall(ToolCallRecord {
                id: "call_1".to_string(),
                name: "tool.discovery.list".to_string(),
                arguments: "{}".to_string(),
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "call_1".to_string(),
                name: Some("tool.discovery.list".to_string()),
                result: "[]".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];

        let body = OpenAiResponsesEngine::map_request(&request);

        assert_eq!(body["tools"][0]["name"], "tool_discovery_list");
        assert_eq!(body["input"][1]["type"], "function_call");
        assert_eq!(body["input"][1]["name"], "tool_discovery_list");
        assert_eq!(body["input"][2]["type"], "function_call_output");
    }

    #[test]
    fn skips_tool_calls_without_matching_outputs() {
        let mut request = request();
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("Run a tool")),
            TranscriptItem::ToolCall(ToolCallRecord {
                id: "call_orphan".to_string(),
                name: "echo".to_string(),
                arguments: "{}".to_string(),
            }),
            TranscriptItem::UserMessage(UserMessage::text("continue")),
        ];

        let body = OpenAiResponsesEngine::map_request(&request);
        let input = body["input"].as_array().unwrap();

        assert_eq!(
            input
                .iter()
                .filter(|item| item["type"] == "function_call")
                .count(),
            0
        );
        assert_eq!(input.len(), 2);
        assert_eq!(input[1]["role"], "user");
    }

    #[test]
    fn skips_tool_results_without_matching_calls() {
        let mut request = request();
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("continue")),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "call_orphan".to_string(),
                name: Some("echo".to_string()),
                result: "stale output".to_string(),
                display_payload: None,
                is_error: false,
            }),
            TranscriptItem::UserMessage(UserMessage::text("continue again")),
        ];

        let body = OpenAiResponsesEngine::map_request(&request);
        let input = body["input"].as_array().unwrap();

        assert_eq!(
            input
                .iter()
                .filter(|item| item["type"] == "function_call_output")
                .count(),
            0
        );
        assert_eq!(input.len(), 2);
        assert_eq!(input[1]["role"], "user");
    }

    mod image_replay_tests {
        include!("image_replay_tests.rs");
    }

    #[test]
    fn skips_provider_function_calls_without_matching_outputs() {
        let mut request = request();
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("Run a tool")),
            TranscriptItem::ProviderMetadata(json!({
                "output": [
                    {
                        "id": "fc_orphan",
                        "type": "function_call",
                        "status": "completed",
                        "call_id": "call_orphan",
                        "name": "echo",
                        "arguments": "{}"
                    }
                ]
            })),
            TranscriptItem::ToolCall(ToolCallRecord {
                id: "call_orphan".to_string(),
                name: "echo".to_string(),
                arguments: "{}".to_string(),
            }),
            TranscriptItem::UserMessage(UserMessage::text("continue")),
        ];

        let body = OpenAiResponsesEngine::map_request(&request);
        let input = body["input"].as_array().unwrap();

        assert_eq!(
            input
                .iter()
                .filter(|item| item["type"] == "function_call")
                .count(),
            0
        );
        assert_eq!(input.len(), 2);
        assert_eq!(input[1]["role"], "user");
    }

    #[test]
    fn replays_provider_function_call_names_as_openai_safe_names() {
        let mut request = request();
        request.tools = vec![roder_api::tools::ToolSpec {
            name: "tool.discovery.list".to_string(),
            description: "list discovery tools".to_string(),
            parameters: json!({ "type": "object" }),
        }];
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("List discovery tools")),
            TranscriptItem::ProviderMetadata(json!({
                "output": [
                    {
                        "id": "fc_1",
                        "type": "function_call",
                        "status": "completed",
                        "call_id": "call_1",
                        "name": "tool.discovery.list",
                        "arguments": "{}"
                    }
                ]
            })),
            TranscriptItem::ToolCall(ToolCallRecord {
                id: "call_1".to_string(),
                name: "tool.discovery.list".to_string(),
                arguments: "{}".to_string(),
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "call_1".to_string(),
                name: Some("tool.discovery.list".to_string()),
                result: "[]".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];

        let body = OpenAiResponsesEngine::map_request(&request);

        assert_eq!(body["input"][1]["type"], "function_call");
        assert_eq!(body["input"][1]["name"], "tool_discovery_list");
        assert_eq!(
            body["input"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["type"] == "function_call")
                .count(),
            1
        );
        assert_eq!(body["input"][2]["type"], "function_call_output");
    }

    #[test]
    fn hosted_web_search_without_function_tools_remains_available() {
        let mut request = request();
        request.tools.clear();
        request.tool_choice = roder_api::tools::ToolChoice::None;
        request.runtime.hosted_web_search = roder_api::inference::HostedWebSearchConfig::live();

        let body = OpenAiResponsesEngine::map_request(&request);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "web_search");
        assert_eq!(tools[0]["external_web_access"], true);
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn xai_mapping_uses_thread_cache_key_and_omits_encrypted_reasoning() {
        let mut request = request();
        request.model.provider = PROVIDER_XAI.to_string();
        request.model.model = "grok-4.3".to_string();
        request.runtime.prompt_cache_key = Some("openai-cache".to_string());
        request.runtime.hosted_web_search = roder_api::inference::HostedWebSearchConfig::cached();

        let (body, _) = OpenAiResponsesEngine::map_request_with_options(
            &request,
            RequestMappingOptions {
                profile: ResponsesProviderProfile::Xai,
                thread_id: Some("thread-123"),
            },
        );

        assert_eq!(body["prompt_cache_key"], "thread-123");
        assert_eq!(body["reasoning"], json!({ "effort": "medium" }));
        assert!(body.get("include").is_none());

        // For Xai profile (direct xAI and SuperGrok), the web_search tool must
        // not include the "external_web_access" key — xAI rejects it with 400.
        let tools = body["tools"].as_array().expect("tools present");
        let ws = tools
            .iter()
            .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("web_search"))
            .expect("web_search tool present");
        assert!(
            ws.get("external_web_access").is_none(),
            "xAI must not receive external_web_access"
        );
    }

    #[test]
    fn profile_xai_mapping_omits_reasoning_for_non_reasoning_grok_models() {
        let mut request = request();
        request.model.provider = PROVIDER_XAI.to_string();
        request.model.model = "grok-4.20-0309-non-reasoning".to_string();

        let (body, _) = OpenAiResponsesEngine::map_request_with_options(
            &request,
            RequestMappingOptions {
                profile: ResponsesProviderProfile::Xai,
                thread_id: Some("thread-123"),
            },
        );

        assert!(body.get("reasoning").is_none());
        assert!(body.get("include").is_none());
    }

    #[test]
    fn profile_openrouter_preserves_slash_model_and_omits_openai_encrypted_reasoning() {
        let mut request = request();
        request.model.provider = PROVIDER_OPENROUTER.to_string();
        request.model.model = "x-ai/grok-4.6".to_string();
        request.runtime.prompt_cache_key = Some("cache-key".to_string());
        request.instructions.developer =
            Some("Proactive multi-agent delegation is active.".to_string());

        let (body, _) = OpenAiResponsesEngine::map_request_with_options(
            &request,
            RequestMappingOptions {
                profile: ResponsesProviderProfile::OpenRouter,
                thread_id: Some("thread-123"),
            },
        );

        assert_eq!(body["model"], "x-ai/grok-4.6");
        assert_eq!(body["reasoning"], json!({ "effort": "medium" }));
        assert_eq!(body["prompt_cache_key"], "cache-key");
        assert!(body.get("instructions").is_none());
        assert_eq!(body["input"][0]["role"], "system");
        assert_eq!(body["input"][0]["content"][0]["text"], "be helpful");
        assert_eq!(body["input"][1]["role"], "system");
        assert!(
            body["input"][1]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Proactive multi-agent delegation is active")
        );
        assert!(body.get("include").is_none());
    }

    #[test]
    fn xai_errors_explain_auth_and_entitlement_boundaries() {
        let unauthorized = xai_error_message(reqwest::StatusCode::UNAUTHORIZED, "bad key");
        assert!(unauthorized.contains("Check XAI_API_KEY"));
        assert!(unauthorized.contains("roder auth login supergrok"));

        let forbidden = xai_error_message(reqwest::StatusCode::FORBIDDEN, "subscription missing");
        assert!(forbidden.contains("entitlement or quota"));
        assert!(forbidden.contains("X Premium+ may not include"));
        assert!(forbidden.contains("subscription missing"));
    }

    #[test]
    fn openrouter_errors_explain_common_provider_boundaries() {
        let unauthorized =
            openrouter_error_message("OpenAI Responses error 401 Unauthorized: invalid key");
        assert!(unauthorized.contains("OpenRouter auth failed"));
        assert!(unauthorized.contains("OPENROUTER_API_KEY"));

        let credits = openrouter_error_message("OpenAI Responses error 402: insufficient credits");
        assert!(credits.contains("credits or quota"));

        let unsupported = openrouter_error_message(
            "OpenAI Responses error 400: unsupported parameter reasoning.encrypted_content",
        );
        assert!(unsupported.contains("rejected a request parameter"));

        let context =
            openrouter_error_message("OpenAI Responses error 400: context length limit exceeded");
        assert!(context.contains("context length exceeded"));
    }

    #[test]
    fn fireworks_errors_explain_common_provider_boundaries() {
        let unauthorized =
            fireworks_error_message("OpenAI Responses error 401 Unauthorized: invalid key");
        assert!(unauthorized.contains("Fireworks auth failed"));
        assert!(unauthorized.contains("FIREWORKS_API_KEY"));

        let payment = fireworks_error_message("OpenAI Responses error 402: payment required");
        assert!(payment.contains("billing or quota"));

        let unavailable =
            fireworks_error_message("OpenAI Responses error 404: model is not deployed");
        assert!(unavailable.contains("model unavailable"));

        let capacity = fireworks_error_message("OpenAI Responses error 429: capacity exceeded");
        assert!(capacity.contains("rate limit or capacity"));
    }

    #[test]
    fn replays_provider_function_call_items_before_tool_outputs() {
        let mut request = request();
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("List files")),
            TranscriptItem::ProviderMetadata(json!({
                "output": [
                    {
                        "id": "rs_1",
                        "type": "reasoning",
                        "encrypted_content": "encrypted-thinking",
                        "summary": []
                    },
                    {
                        "id": "fc_1",
                        "type": "function_call",
                        "status": "completed",
                        "call_id": "call_1",
                        "name": "list_files",
                        "arguments": "{\"path\":\".\"}"
                    }
                ]
            })),
            TranscriptItem::ToolCall(ToolCallRecord {
                id: "call_1".to_string(),
                name: "list_files".to_string(),
                arguments: "{\"path\":\".\"}".to_string(),
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "call_1".to_string(),
                name: Some("list_files".to_string()),
                result: "Cargo.toml".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];

        let input = input_items(&request);
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[1]["encrypted_content"], "encrypted-thinking");
        assert_eq!(input[2]["id"], "fc_1");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[2]["status"], "completed");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(
            input
                .iter()
                .filter(|item| item["type"] == "function_call")
                .count(),
            1
        );
    }

    #[test]
    fn fireworks_replay_strips_encrypted_provider_reasoning() {
        let mut request = request();
        request.model.provider = PROVIDER_FIREWORKS.to_string();
        request.model.model = "accounts/fireworks/models/qwen3-235b-a22b".to_string();
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("Think briefly")),
            TranscriptItem::ProviderMetadata(json!({
                "output": [
                    {
                        "id": "rs_1",
                        "type": "reasoning",
                        "encrypted_content": "encrypted-thinking",
                        "summary": []
                    }
                ]
            })),
        ];

        let (body, _) = OpenAiResponsesEngine::map_request_with_options(
            &request,
            RequestMappingOptions {
                profile: ResponsesProviderProfile::Fireworks,
                thread_id: Some("thread-123"),
            },
        );

        assert_eq!(body["input"][2]["type"], "reasoning");
        assert!(body["input"][2].get("encrypted_content").is_none());
    }

    #[test]
    fn replays_provider_compaction_items() {
        let mut request = request();
        request.transcript = vec![
            TranscriptItem::ProviderMetadata(json!({
                "output": [
                    {
                        "id": "cmp_1",
                        "type": "compaction",
                        "encrypted_content": "opaque"
                    }
                ]
            })),
            TranscriptItem::UserMessage(UserMessage::text("Continue")),
        ];

        let input = input_items(&request);
        assert_eq!(input[0]["type"], "compaction");
        assert_eq!(input[0]["encrypted_content"], "opaque");
        assert_eq!(input[1]["role"], "user");
    }

    #[test]
    fn drops_pre_compaction_history_when_provider_compaction_exists() {
        let mut request = request();
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("old history that must be dropped")),
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "old answer".to_string(),
                phase: None,
            }),
            TranscriptItem::ToolCall(ToolCallRecord {
                id: "call_old".to_string(),
                name: "read_file".to_string(),
                arguments: "{\"path\":\"old.rs\"}".to_string(),
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "call_old".to_string(),
                name: Some("read_file".to_string()),
                result: "huge tool dump ".repeat(1_000),
                display_payload: None,
                is_error: false,
            }),
            TranscriptItem::ProviderMetadata(json!({
                "output": [
                    {
                        "id": "cmp_1",
                        "type": "compaction",
                        "encrypted_content": "opaque-state"
                    }
                ]
            })),
            TranscriptItem::UserMessage(UserMessage::text("Continue after compact")),
        ];

        let input = input_items(&request);
        assert_eq!(
            input.len(),
            2,
            "expected only compaction + new user: {input:?}"
        );
        assert_eq!(input[0]["type"], "compaction");
        assert_eq!(input[0]["encrypted_content"], "opaque-state");
        assert_eq!(input[1]["role"], "user");
        assert!(
            input[1]["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("Continue after compact")),
            "{input:?}"
        );
        assert!(
            !input.iter().any(|item| {
                item.get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|content| {
                        content.iter().any(|part| {
                            part.get("text")
                                .and_then(Value::as_str)
                                .is_some_and(|text| text.contains("old history"))
                        })
                    })
            }),
            "pre-compaction user history must not be replayed: {input:?}"
        );
    }

    #[test]
    fn fallback_function_call_items_use_responses_item_id_prefix() {
        let mut request = request();
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("List files")),
            TranscriptItem::ToolCall(ToolCallRecord {
                id: "call_1".to_string(),
                name: "list_files".to_string(),
                arguments: "{\"path\":\".\"}".to_string(),
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "call_1".to_string(),
                name: Some("list_files".to_string()),
                result: "Cargo.toml".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];

        let input = input_items(&request);
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["id"], "fc_1");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[2]["type"], "function_call_output");
    }

    #[test]
    fn extracts_responses_output_text() {
        let value = json!({
            "output": [{ "content": [{ "text": "hello" }, { "output_text": " world" }] }]
        });
        assert_eq!(extract_output_text(&value).unwrap(), "hello world");
    }

    #[test]
    fn ignores_non_final_responses_output_text() {
        let value = json!({
            "output": [
                { "phase": "analysis", "content": [{ "text": "hidden" }] },
                { "phase": "final_answer", "content": [{ "text": "shown" }] }
            ]
        });
        assert_eq!(extract_response_text(&value), "shown");
    }

    #[test]
    fn extracts_responses_usage() {
        let value = json!({
            "usage": {
                "input_tokens": 10,
                "output_tokens": 4,
                "input_tokens_details": { "cached_tokens": 9 }
            }
        });
        assert_eq!(
            extract_usage(&value),
            Some(TokenUsage::new(10, 4, 14).with_cached_prompt_tokens(9))
        );
    }

    #[test]
    fn extracts_the_service_tier_openai_served() {
        let value = json!({
            "service_tier": "default",
            "usage": { "input_tokens": 10, "output_tokens": 4 }
        });
        assert_eq!(
            extract_usage(&value).and_then(|usage| usage.service_tier),
            Some("default".to_string())
        );
    }

    #[test]
    fn sends_service_tier_only_to_openai_when_requested() {
        let mut request = request();
        let body = OpenAiResponsesEngine::map_request_with_options(
            &request,
            RequestMappingOptions::default(),
        )
        .0;
        assert!(body.get("service_tier").is_none());

        request.runtime.service_tier = Some("priority".to_string());
        let body = OpenAiResponsesEngine::map_request_with_options(
            &request,
            RequestMappingOptions::default(),
        )
        .0;
        assert_eq!(body["service_tier"], json!("priority"));

        for profile in [
            ResponsesProviderProfile::OpenRouter,
            ResponsesProviderProfile::Xai,
            ResponsesProviderProfile::Fireworks,
        ] {
            let body = OpenAiResponsesEngine::map_request_with_options(
                &request,
                RequestMappingOptions {
                    profile,
                    thread_id: None,
                },
            )
            .0;
            assert!(
                body.get("service_tier").is_none(),
                "{profile:?} must not receive service_tier"
            );
        }
    }

    #[test]
    fn extracts_responses_tool_calls() {
        let value = json!({
            "output": [{
                "type": "function_call",
                "call_id": "call_1",
                "name": "echo",
                "arguments": "{\"text\":\"hello\"}"
            }]
        });
        let calls = extract_tool_calls(&value, &HashMap::new());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "echo");
        assert_eq!(calls[0].arguments, "{\"text\":\"hello\"}");
    }

    #[test]
    fn remaps_api_tool_names_to_canonical_names_from_completed_output() {
        let mut map = HashMap::new();
        map.insert("memory_save".to_string(), "memory.save".to_string());
        let value = json!({
            "output": [{
                "type": "function_call",
                "call_id": "call_1",
                "name": "memory_save",
                "arguments": "{\"entry\":\"x\"}"
            }]
        });
        let calls = extract_tool_calls(&value, &map);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "memory.save");
    }

    #[test]
    fn remaps_api_tool_names_to_canonical_names_in_output_item_added() {
        let mut state = ResponsesStreamState::default();
        state
            .tool_name_map
            .insert("memory_save".to_string(), "memory.save".to_string());

        let added = SseEvent {
            event: Some("response.output_item.done".to_string()),
            data: json!({
                "type": "response.output_item.done",
                "item": {
                    "id": "fc_1",
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "memory_save",
                    "arguments": "{\"entry\":\"x\"}"
                }
            }),
        };
        let events = events_from_sse_event(&added, &mut state);
        assert_eq!(
            events,
            vec![
                InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_1".to_string(),
                    name: "memory.save".to_string(),
                    arguments: "{\"entry\":\"x\"}".to_string(),
                }),
                InferenceEvent::OutputItemCompleted(added.data["item"].clone())
            ]
        );
    }

    #[test]
    fn parses_responses_sse_data_frames() {
        let frame = "event: response.output_text.delta\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\r\n";
        let event = parse_sse_frame(frame).unwrap().unwrap();
        assert_eq!(event.event.as_deref(), Some("response.output_text.delta"));
        assert_eq!(event.data["delta"], "hi");
        assert_eq!(parse_sse_frame("data: [DONE]\n").unwrap(), None);
    }

    #[test]
    fn emits_streaming_text_and_completed_metadata() {
        let mut state = ResponsesStreamState::default();
        let delta = SseEvent {
            event: Some("response.output_text.delta".to_string()),
            data: json!({ "type": "response.output_text.delta", "delta": "hello" }),
        };
        let events = events_from_sse_event(&delta, &mut state);
        assert_eq!(
            events,
            vec![InferenceEvent::MessageDelta(MessageDelta {
                text: "hello".to_string(),
                phase: None,
            })]
        );

        let completed = SseEvent {
            event: Some("response.completed".to_string()),
            data: json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "status": "completed",
                    "output_text": "hello",
                    "usage": {
                        "input_tokens": 3,
                        "output_tokens": 4,
                        "total_tokens": 7
                    }
                }
            }),
        };
        let events = events_from_sse_event(&completed, &mut state);
        assert!(state.terminal);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, InferenceEvent::MessageDelta(_)))
        );
        assert!(events.iter().any(|event| {
            matches!(
                event,
                InferenceEvent::Usage(usage)
                    if *usage == TokenUsage::new(3, 4, 7).with_cached_prompt_tokens(0)
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                InferenceEvent::Completed(CompletionMetadata {
                    provider_response_id: Some(id),
                    ..
                }) if id == "resp_1"
            )
        }));
    }

    #[test]
    fn emits_completed_text_when_no_delta_was_streamed() {
        let mut state = ResponsesStreamState::default();
        let event = SseEvent {
            event: None,
            data: json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "output_text": "fallback"
                }
            }),
        };
        let events = events_from_sse_event(&event, &mut state);
        assert!(matches!(
            events.first(),
            Some(InferenceEvent::MessageDelta(MessageDelta { text, phase })) if text == "fallback" && phase.as_deref() == Some("final_answer")
        ));
    }

    #[test]
    fn emits_phase_text_deltas() {
        let mut state = ResponsesStreamState::default();
        let added = SseEvent {
            event: Some("response.output_item.added".to_string()),
            data: json!({
                "type": "response.output_item.added",
                "item": {
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "phase": "commentary"
                }
            }),
        };
        assert!(events_from_sse_event(&added, &mut state).is_empty());

        let commentary_delta = SseEvent {
            event: Some("response.output_text.delta".to_string()),
            data: json!({
                "type": "response.output_text.delta",
                "item_id": "msg_1",
                "delta": "I will inspect first."
            }),
        };
        assert_eq!(
            events_from_sse_event(&commentary_delta, &mut state),
            vec![InferenceEvent::MessageDelta(MessageDelta {
                text: "I will inspect first.".to_string(),
                phase: Some("commentary".to_string()),
            })]
        );

        let final_added = SseEvent {
            event: Some("response.output_item.added".to_string()),
            data: json!({
                "type": "response.output_item.added",
                "item": {
                    "id": "msg_2",
                    "type": "message",
                    "role": "assistant",
                    "phase": "final_answer"
                }
            }),
        };
        assert!(events_from_sse_event(&final_added, &mut state).is_empty());

        let final_delta = SseEvent {
            event: Some("response.output_text.delta".to_string()),
            data: json!({
                "type": "response.output_text.delta",
                "item_id": "msg_2",
                "delta": "Done."
            }),
        };
        assert_eq!(
            events_from_sse_event(&final_delta, &mut state),
            vec![InferenceEvent::MessageDelta(MessageDelta {
                text: "Done.".to_string(),
                phase: Some("final_answer".to_string()),
            })]
        );
    }

    #[test]
    fn emits_commentary_from_done_item_when_no_delta_was_streamed() {
        let mut state = ResponsesStreamState::default();
        let done = SseEvent {
            event: Some("response.output_item.done".to_string()),
            data: json!({
                "type": "response.output_item.done",
                "item": {
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "phase": "commentary",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "I’ll inspect the logs first."
                        }
                    ]
                }
            }),
        };

        assert_eq!(
            events_from_sse_event(&done, &mut state),
            vec![
                InferenceEvent::MessageDelta(MessageDelta {
                    text: "I’ll inspect the logs first.".to_string(),
                    phase: Some("commentary".to_string()),
                }),
                InferenceEvent::OutputItemCompleted(done.data["item"].clone())
            ]
        );
    }

    #[test]
    fn emits_commentary_from_done_item_with_string_content() {
        let mut state = ResponsesStreamState::default();
        let done = SseEvent {
            event: Some("response.output_item.done".to_string()),
            data: json!({
                "type": "response.output_item.done",
                "item": {
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "phase": "commentary",
                    "content": "I’ll inspect the logs and then summarize root cause."
                }
            }),
        };

        assert_eq!(
            events_from_sse_event(&done, &mut state),
            vec![
                InferenceEvent::MessageDelta(MessageDelta {
                    text: "I’ll inspect the logs and then summarize root cause.".to_string(),
                    phase: Some("commentary".to_string()),
                }),
                InferenceEvent::OutputItemCompleted(done.data["item"].clone())
            ]
        );
    }

    #[test]
    fn skips_done_final_message_after_idless_streamed_final_delta() {
        let mut state = ResponsesStreamState::default();
        let delta = SseEvent {
            event: Some("response.output_text.delta".to_string()),
            data: json!({
                "type": "response.output_text.delta",
                "delta": "Hello from the streamed response."
            }),
        };
        assert_eq!(
            events_from_sse_event(&delta, &mut state),
            vec![InferenceEvent::MessageDelta(MessageDelta {
                text: "Hello from the streamed response.".to_string(),
                phase: None,
            })]
        );

        let done = SseEvent {
            event: Some("response.output_item.done".to_string()),
            data: json!({
                "type": "response.output_item.done",
                "item": {
                    "id": "msg_final",
                    "type": "message",
                    "role": "assistant",
                    "phase": "final_answer",
                    "content": [{
                        "type": "output_text",
                        "text": "Hello from the streamed response."
                    }]
                }
            }),
        };
        assert_eq!(
            events_from_sse_event(&done, &mut state),
            vec![InferenceEvent::OutputItemCompleted(
                done.data["item"].clone()
            )]
        );
    }

    #[test]
    fn parse_sse_frame_error_includes_raw_data_excerpt() {
        let err = parse_sse_frame("event: response.output_item.done\ndata: {\"ok\":true} trailing")
            .unwrap_err()
            .to_string();

        assert!(err.contains("failed to parse Responses SSE data as JSON"));
        assert!(err.contains("{\"ok\":true} trailing"));
    }

    #[test]
    fn emits_all_unstreamed_phase_messages_from_completed_response() {
        let mut state = ResponsesStreamState::default();
        let completed = SseEvent {
            event: Some("response.completed".to_string()),
            data: json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "status": "completed",
                    "output": [
                        {
                            "id": "msg_1",
                            "type": "message",
                            "role": "assistant",
                            "phase": "commentary",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "I’ll inspect first."
                                }
                            ]
                        },
                        {
                            "id": "msg_2",
                            "type": "message",
                            "role": "assistant",
                            "phase": "final_answer",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "Done."
                                }
                            ]
                        }
                    ],
                    "usage": {
                        "input_tokens": 1,
                        "output_tokens": 2,
                        "total_tokens": 3
                    }
                }
            }),
        };

        let events = events_from_sse_event(&completed, &mut state);

        assert!(events.iter().any(|event| matches!(
            event,
            InferenceEvent::MessageDelta(MessageDelta { text, phase })
                if text == "I’ll inspect first." && phase.as_deref() == Some("commentary")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            InferenceEvent::MessageDelta(MessageDelta { text, phase })
                if text == "Done." && phase.as_deref() == Some("final_answer")
        )));
    }

    #[test]
    fn emits_reasoning_text_deltas_and_avoids_done_duplicates() {
        let mut state = ResponsesStreamState::default();
        let delta = SseEvent {
            event: Some("response.reasoning_text.delta".to_string()),
            data: json!({
                "type": "response.reasoning_text.delta",
                "item_id": "rs_1",
                "content_index": 0,
                "delta": "The user is asking "
            }),
        };
        assert_eq!(
            events_from_sse_event(&delta, &mut state),
            vec![InferenceEvent::ReasoningDelta(ReasoningDelta {
                text: "The user is asking ".to_string(),
            })]
        );

        let done = SseEvent {
            event: Some("response.reasoning_text.done".to_string()),
            data: json!({
                "type": "response.reasoning_text.done",
                "item_id": "rs_1",
                "content_index": 0,
                "text": "The user is asking for visible thinking."
            }),
        };
        assert!(events_from_sse_event(&done, &mut state).is_empty());
    }

    #[test]
    fn emits_reasoning_text_done_when_no_delta_was_streamed() {
        let mut state = ResponsesStreamState::default();
        let done = SseEvent {
            event: Some("response.reasoning_summary_text.done".to_string()),
            data: json!({
                "type": "response.reasoning_summary_text.done",
                "item_id": "rs_1",
                "content_index": 0,
                "text": "I should inspect the repo."
            }),
        };

        assert_eq!(
            events_from_sse_event(&done, &mut state),
            vec![InferenceEvent::ReasoningDelta(ReasoningDelta {
                text: "I should inspect the repo.".to_string(),
            })]
        );
    }

    #[test]
    fn function_arguments_done_waits_for_completed_output_item() {
        let mut state = ResponsesStreamState::default();
        let added = SseEvent {
            event: Some("response.output_item.added".to_string()),
            data: json!({
                "type": "response.output_item.added",
                "item": {
                    "id": "fc_1",
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "echo"
                }
            }),
        };
        assert_eq!(
            events_from_sse_event(&added, &mut state),
            vec![InferenceEvent::ToolCallStarted(ToolCallStarted {
                id: "call_1".to_string(),
                name: "echo".to_string(),
            })]
        );

        let delta = SseEvent {
            event: Some("response.function_call_arguments.delta".to_string()),
            data: json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "fc_1",
                "delta": "{\"text\":"
            }),
        };
        assert_eq!(
            events_from_sse_event(&delta, &mut state),
            vec![InferenceEvent::ToolCallDelta(ToolCallDelta {
                id: "call_1".to_string(),
                arguments_delta: "{\"text\":".to_string(),
            })]
        );

        let done = SseEvent {
            event: Some("response.function_call_arguments.done".to_string()),
            data: json!({
                "type": "response.function_call_arguments.done",
                "item_id": "fc_1",
                "name": "echo",
                "arguments": "{\"text\":\"hello\"}"
            }),
        };
        assert!(events_from_sse_event(&done, &mut state).is_empty());
        let completed = SseEvent {
            event: None,
            data: json!({
                "type": "response.output_item.done", "item": {
                    "id":"fc_1", "type":"function_call", "call_id":"call_1", "name":"echo"
                }
            }),
        };
        let events = events_from_sse_event(&completed, &mut state);
        assert!(matches!(&events[0], InferenceEvent::ToolCallCompleted(call)
            if call.id == "call_1" && call.arguments == "{\"text\":\"hello\"}"));
    }

    #[test]
    fn emits_hosted_web_search_tool_events() {
        let mut state = ResponsesStreamState::default();
        let added = SseEvent {
            event: Some("response.output_item.added".to_string()),
            data: json!({
                "type": "response.output_item.added",
                "item": {
                    "id": "ws_1",
                    "type": "web_search_call",
                    "status": "in_progress"
                }
            }),
        };
        assert_eq!(
            events_from_sse_event(&added, &mut state),
            vec![InferenceEvent::HostedToolCallStarted(
                HostedToolCallStarted {
                    id: "ws_1".to_string(),
                    name: "web_search".to_string(),
                }
            )]
        );

        let done = SseEvent {
            event: Some("response.output_item.done".to_string()),
            data: json!({
                "type": "response.output_item.done",
                "item": {
                    "id": "ws_1",
                    "type": "web_search_call",
                    "status": "completed",
                    "action": {
                        "type": "search",
                        "query": "pandelis zembashis"
                    }
                }
            }),
        };
        assert_eq!(
            events_from_sse_event(&done, &mut state),
            vec![
                InferenceEvent::HostedToolCallCompleted(HostedToolCallCompleted {
                    id: "ws_1".to_string(),
                    name: "web_search".to_string(),
                    arguments: r#"{"action":"search","query":"pandelis zembashis"}"#.to_string(),
                }),
                InferenceEvent::OutputItemCompleted(done.data["item"].clone())
            ]
        );
    }

    #[test]
    fn emits_unstreamed_hosted_web_search_from_completed_response() {
        let mut state = ResponsesStreamState::default();
        let completed = SseEvent {
            event: Some("response.completed".to_string()),
            data: json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "status": "completed",
                    "output": [
                        {
                            "id": "ws_1",
                            "type": "web_search_call",
                            "status": "completed",
                            "action": {
                                "type": "search",
                                "queries": ["pandelis zembashis"]
                            }
                        }
                    ],
                    "usage": {
                        "input_tokens": 1,
                        "output_tokens": 2,
                        "total_tokens": 3
                    }
                }
            }),
        };

        let events = events_from_sse_event(&completed, &mut state);

        assert!(events.iter().any(|event| matches!(
            event,
            InferenceEvent::HostedToolCallStarted(HostedToolCallStarted { id, name })
                if id == "ws_1" && name == "web_search"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            InferenceEvent::HostedToolCallCompleted(HostedToolCallCompleted {
                id,
                name,
                arguments
            }) if id == "ws_1"
                && name == "web_search"
                && arguments == r#"{"action":"search","query":"pandelis zembashis"}"#
        )));
    }

    #[test]
    fn emits_compaction_progress_for_compaction_output_items() {
        let mut state = ResponsesStreamState::default();
        let added = SseEvent {
            event: Some("response.output_item.added".to_string()),
            data: json!({
                "type": "response.output_item.added",
                "item": {
                    "id": "cmp_1",
                    "type": "compaction",
                    "encrypted_content": "opaque"
                }
            }),
        };
        let expected_item = json!({
            "id": "cmp_1",
            "type": "compaction",
            "encrypted_content": "opaque"
        });
        assert_eq!(
            events_from_sse_event(&added, &mut state),
            vec![InferenceEvent::Compaction(CompactionProgress {
                status: "started".to_string(),
                item_id: Some("cmp_1".to_string()),
                tokens_before: None,
                tokens_after: None,
                duration_ms: None,
                item: Some(expected_item.clone()),
            })]
        );

        let done = SseEvent {
            event: Some("response.output_item.done".to_string()),
            data: json!({
                "type": "response.output_item.done",
                "item": {
                    "id": "cmp_1",
                    "type": "compaction",
                    "encrypted_content": "opaque"
                }
            }),
        };
        assert_eq!(
            events_from_sse_event(&done, &mut state),
            vec![InferenceEvent::Compaction(CompactionProgress {
                status: "completed".to_string(),
                item_id: Some("cmp_1".to_string()),
                tokens_before: None,
                tokens_after: None,
                duration_ms: None,
                item: Some(expected_item),
            })]
        );
    }

    #[test]
    fn emits_stream_failures() {
        let mut state = ResponsesStreamState::default();
        let event = SseEvent {
            event: None,
            data: json!({
                "type": "response.failed",
                "response": {
                    "error": { "message": "bad stream" }
                }
            }),
        };
        let events = events_from_sse_event(&event, &mut state);
        assert!(state.terminal);
        assert_eq!(
            events,
            vec![InferenceEvent::Failed(InferenceFailure {
                message: "bad stream".to_string(),
            })]
        );
    }
}

#[cfg(test)]
#[path = "audit_regressions.rs"]
mod audit_regressions;

#[cfg(test)]
#[path = "patch_stream_tests.rs"]
mod patch_stream_tests;
