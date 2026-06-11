use roder_api::catalog::{
    PROVIDER_OPENAI, PROVIDER_OPENROUTER, PROVIDER_SUPERGROK, PROVIDER_XAI, models_for_provider,
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
}

impl OpenAiResponsesEngine {
    pub fn new(api_key: String) -> Self {
        Self::new_with_provider_id(api_key, PROVIDER_OPENAI)
    }

    pub fn new_with_provider_id(api_key: String, provider_id: impl Into<String>) -> Self {
        Self::new_with_config(
            api_key,
            provider_id,
            "https://api.openai.com/v1",
            Vec::new(),
        )
    }

    pub fn new_with_config(
        api_key: String,
        provider_id: impl Into<String>,
        base_url: impl Into<String>,
        headers: Vec<(String, String)>,
    ) -> Self {
        let provider_id = provider_id.into();
        let profile = match provider_id.as_str() {
            PROVIDER_XAI | PROVIDER_SUPERGROK => ResponsesProviderProfile::Xai,
            PROVIDER_OPENROUTER => ResponsesProviderProfile::OpenRouter,
            _ => ResponsesProviderProfile::OpenAi,
        };
        let display_name = match provider_id.as_str() {
            PROVIDER_XAI => "xAI".to_string(),
            PROVIDER_OPENROUTER => "OpenRouter".to_string(),
            _ => "OpenAI".to_string(),
        };
        Self {
            api_key: Some(api_key),
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
        let (tools, tool_name_map) = responses_tools(request);
        let input = response_input_items_with_options(request, &tool_name_map, options.profile);
        let mut body = json!({
            "model": request.model.model,
            "input": input,
            "store": false,
            "stream": true,
        });
        if options.profile != ResponsesProviderProfile::OpenRouter
            && let Some(system) = request
                .instructions
                .system
                .as_deref()
                .filter(|s| !s.is_empty())
        {
            body["instructions"] = json!(system);
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
                        Some(level) => json!({ "effort": level, "summary": "auto" }),
                        None => json!({ "summary": "auto" }),
                    };
                    body["include"] = json!(["reasoning.encrypted_content"]);
                }
                ResponsesProviderProfile::Xai => {}
            }
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
        let prompt_cache_key = if options.profile == ResponsesProviderProfile::Xai {
            options.thread_id.filter(|thread_id| !thread_id.is_empty())
        } else {
            request.runtime.prompt_cache_key.as_deref()
        };
        if let Some(prompt_cache_key) = prompt_cache_key {
            body["prompt_cache_key"] = json!(prompt_cache_key);
        }
        if let Some(threshold) = request
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
    model == "grok-4.3"
        || model == "grok-4.20-0309-reasoning"
        || model.starts_with("grok-4.20-multi-agent-")
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
    #[serde(default)]
    context_length: Option<u32>,
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
            (!id.is_empty()).then(|| ModelDescriptor {
                id: id.to_string(),
                name: model.name.unwrap_or_else(|| id.to_string()),
                context_window: model.context_length,
                default_reasoning: None,
                supported_reasoning: Vec::new(),
            })
        })
        .collect()
}

fn cached_models(provider_id: &str, base_url: &str) -> anyhow::Result<CachedProviderModels> {
    let cache: ModelsCacheFile = serde_json::from_str(&fs::read_to_string(cache_path())?)?;
    cache
        .providers
        .get(provider_id)
        .filter(|entry| entry.base_url.trim_end_matches('/') == base_url.trim_end_matches('/'))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no cached models for {provider_id}"))
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

fn responses_tools(request: &AgentInferenceRequest) -> (Vec<Value>, ResponsesToolNameMap) {
    let mut tools = Vec::new();
    let mut used_tool_names = HashSet::new();
    let mut tool_name_map = ResponsesToolNameMap::default();
    match request.runtime.hosted_web_search.mode {
        HostedWebSearchMode::Disabled => {}
        HostedWebSearchMode::Cached => tools.push(json!({
            "type": "web_search",
            "external_web_access": false,
        })),
        HostedWebSearchMode::Live => tools.push(json!({
            "type": "web_search",
            "external_web_access": true,
        })),
    }
    for tool in &request.tools {
        let tool = tool.normalized_for_model(roder_api::ToolSchemaPolicy::warning());
        let api_name = responses_tool_name(&tool.name, &mut used_tool_names);
        tool_name_map.register(&tool.name, &api_name);
        let mut entry = json!({
            "type": "function",
            "name": api_name,
            "description": tool.description,
            "parameters": tool.parameters,
        });
        if openai_provider_native_tool_search(request) {
            entry["defer_loading"] = json!(true);
        }
        tools.push(entry);
    }
    if openai_provider_native_tool_search(request) && !request.tools.is_empty() {
        tools.push(json!({ "type": "tool_search" }));
    }
    (tools, tool_name_map)
}

fn openai_provider_native_tool_search(request: &AgentInferenceRequest) -> bool {
    request.runtime.tool_search.is_provider_native_requested()
        && request.model.provider == PROVIDER_OPENAI
        && openai_model_supports_tool_search(&request.model.model)
}

/**
 * Whether an OpenAI model id is known to support Responses `tool_search`.
 * Public so offline eval fixtures exercise the same support gating as the
 * live request mapping.
 */
pub fn openai_model_supports_tool_search(model: &str) -> bool {
    model.starts_with("gpt-5.4") || model.starts_with("gpt-5.5") || model.starts_with("gpt-5.6")
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

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let Some(api_key) = self.api_key.as_ref() else {
            anyhow::bail!(
                "{} API key is missing; configure it from the provider menu or user config",
                self.display_name
            )
        };
        let (body, tool_name_map) = Self::map_request_with_options(
            &request,
            RequestMappingOptions {
                profile: self.profile,
                thread_id: Some(_ctx.thread_id),
            },
        );
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
            ResponsesProviderProfile::Xai => anyhow::anyhow!("xAI Responses error: {err}"),
            ResponsesProviderProfile::OpenRouter => {
                anyhow::anyhow!("{}", openrouter_error_message(&err.to_string()))
            }
            ResponsesProviderProfile::OpenAi => err,
        })?;
        // Provider-native tool search may require client-executed searches
        // against the runtime catalog within the same turn.
        let continuation = openai_provider_native_tool_search(&request).then(|| {
            ClientToolSearchContext {
                base_url: self.base_url.clone(),
                api_key: api_key.clone(),
                headers: self.headers.clone(),
                grok_conversation_id: (self.profile == ResponsesProviderProfile::Xai)
                    .then(|| _ctx.thread_id.to_string()),
                body: body.clone(),
                policy: request.runtime.reliability.clone(),
                catalog: roder_api::tool_search_catalog::ToolSearchCatalog::build(
                    &request.tools,
                    &request.runtime.tool_search,
                ),
            }
        });
        Ok(stream_responses_sse_with_client_tool_search(
            response.response,
            tool_name_map.api_name_to_tool_name,
            response.retry_events,
            continuation,
        ))
    }
}

struct RetriedResponse {
    response: reqwest::Response,
    retry_events: Vec<Value>,
}

async fn send_responses_request(
    base_url: &str,
    api_key: &str,
    headers: &[(String, String)],
    grok_conversation_id: Option<&str>,
    body: &Value,
    policy: Option<&ReliabilityRequestPolicy>,
) -> anyhow::Result<RetriedResponse> {
    send_responses_request_with_idle_timeout(
        base_url,
        api_key,
        headers,
        grok_conversation_id,
        body,
        policy,
        responses_stream_idle_timeout(),
    )
    .await
}

async fn send_responses_request_with_idle_timeout(
    base_url: &str,
    api_key: &str,
    headers: &[(String, String)],
    grok_conversation_id: Option<&str>,
    body: &Value,
    policy: Option<&ReliabilityRequestPolicy>,
    idle_timeout: Duration,
) -> anyhow::Result<RetriedResponse> {
    let policy = policy.cloned().unwrap_or_default();
    let attempts = policy.provider_retry_max_attempts.max(1);
    let client = responses_stream_client(idle_timeout)?;
    let mut last_error = None;
    let mut retry_events = Vec::new();
    let mut body = body.clone();
    let mut recovered_missing_tool_output_call_ids = HashSet::new();
    for attempt in 1..=attempts {
        let mut request = client
            .post(format!("{}/responses", base_url))
            .bearer_auth(api_key);
        for (key, value) in headers {
            request = request.header(key, value);
        }
        if let Some(thread_id) = grok_conversation_id.filter(|id| !id.is_empty()) {
            request = request.header("x-grok-conv-id", thread_id);
        }
        let response = tokio::time::timeout(idle_timeout, request.json(&body).send()).await;
        match response {
            Ok(Ok(response)) if response.status().is_success() => {
                return Ok(RetriedResponse {
                    response,
                    retry_events,
                });
            }
            Ok(Ok(response)) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                let retryable = policy
                    .provider_retry_status_codes
                    .contains(&status.as_u16());
                last_error = Some(format!("OpenAI Responses error {status}: {text}"));
                if status == reqwest::StatusCode::BAD_REQUEST
                    && attempt < attempts
                    && let Some(call_id) = missing_function_call_output_call_id(&text)
                    && recovered_missing_tool_output_call_ids.insert(call_id.clone())
                    && remove_function_call_output(&mut body, &call_id)
                {
                    push_retry_event(
                        &mut retry_events,
                        attempt,
                        "missing_function_call_output_call_id",
                        &policy,
                    );
                    continue;
                }
                if retryable && attempt < attempts {
                    push_retry_event(
                        &mut retry_events,
                        attempt,
                        &provider_retry_status_cause(status.as_u16()),
                        &policy,
                    );
                    retry_sleep(&policy, attempt).await;
                    continue;
                }
            }
            Ok(Err(err)) => {
                let timed_out = err.is_timeout();
                last_error = Some(if timed_out {
                    format!(
                        "OpenAI Responses request timed out after {}ms: {err}",
                        idle_timeout.as_millis()
                    )
                } else {
                    err.to_string()
                });
                if attempt < attempts {
                    let cause = if timed_out {
                        "transport_timeout"
                    } else {
                        "transport_error"
                    };
                    push_retry_event(&mut retry_events, attempt, cause, &policy);
                    retry_sleep(&policy, attempt).await;
                    continue;
                }
            }
            Err(_) => {
                last_error = Some(format!(
                    "OpenAI Responses request timed out waiting for response headers after {}ms",
                    idle_timeout.as_millis()
                ));
                if attempt < attempts {
                    push_retry_event(
                        &mut retry_events,
                        attempt,
                        "response_headers_timeout",
                        &policy,
                    );
                    retry_sleep(&policy, attempt).await;
                    continue;
                }
            }
        }
        break;
    }
    anyhow::bail!(last_error.unwrap_or_else(|| "OpenAI Responses request failed".to_string()))
}

fn missing_function_call_output_call_id(body: &str) -> Option<String> {
    if !body.contains("No tool call found for function call output with call_id") {
        return None;
    }

    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .and_then(extract_missing_function_call_output_call_id)
        })
        .or_else(|| extract_missing_function_call_output_call_id(body))
}

fn extract_missing_function_call_output_call_id(message: &str) -> Option<String> {
    const PREFIX: &str = "No tool call found for function call output with call_id ";
    let tail = message.split_once(PREFIX)?.1;
    let call_id = tail
        .trim_start()
        .trim_end_matches('.')
        .split(|ch: char| ch.is_whitespace() || ch == '.' || ch == ',' || ch == '}' || ch == '"')
        .next()
        .unwrap_or_default()
        .trim();
    (!call_id.is_empty()).then(|| call_id.to_string())
}

fn remove_function_call_output(body: &mut Value, call_id: &str) -> bool {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let before = input.len();
    input.retain(|item| {
        !(item.get("type").and_then(Value::as_str) == Some("function_call_output")
            && item.get("call_id").and_then(Value::as_str) == Some(call_id))
    });
    input.len() != before
}

fn responses_stream_client(idle_timeout: Duration) -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .read_timeout(idle_timeout)
        .build()?)
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

fn stream_responses_sse(
    response: reqwest::Response,
    tool_name_map: HashMap<String, String>,
    retry_events: Vec<Value>,
) -> InferenceEventStream {
    stream_responses_sse_with_client_tool_search(response, tool_name_map, retry_events, None)
}

/// Bounded number of in-turn client-executed tool-search continuations.
const MAX_CLIENT_TOOL_SEARCH_ROUNDS: usize = 3;

/**
 * Connection/request context for the client-executed `tool_search_call` →
 * `tool_search_output` flow (roadmap phase 79): when the model emits a
 * `tool_search_call` item without provider-side results, the client runs
 * the search against the runtime catalog adapter and continues the same
 * turn with a follow-up request carrying the `tool_search_output` item.
 */
struct ClientToolSearchContext {
    base_url: String,
    api_key: String,
    headers: Vec<(String, String)>,
    grok_conversation_id: Option<String>,
    body: Value,
    policy: Option<ReliabilityRequestPolicy>,
    catalog: roder_api::tool_search_catalog::ToolSearchCatalog,
}

fn stream_responses_sse_with_client_tool_search(
    response: reqwest::Response,
    tool_name_map: HashMap<String, String>,
    retry_events: Vec<Value>,
    mut continuation: Option<ClientToolSearchContext>,
) -> InferenceEventStream {
    Box::pin(async_stream::try_stream! {
        use futures::StreamExt as _;

        for retry_event in retry_events {
            yield InferenceEvent::ProviderMetadata(retry_event);
        }

        let mut response = response;
        for _round in 0..=MAX_CLIENT_TOOL_SEARCH_ROUNDS {
            let mut chunks = response.bytes_stream();
            let mut buffer = String::new();
            let mut state = ResponsesStreamState {
                tool_name_map: tool_name_map.clone(),
                ..Default::default()
            };

            while let Some(chunk) = chunks.next().await {
                let chunk = chunk?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some((frame, consumed)) = take_sse_frame(&buffer) {
                    buffer.drain(..consumed);
                    let Some(event) = parse_sse_frame(&frame)? else {
                        continue;
                    };
                    for inference_event in events_from_sse_event(&event, &mut state) {
                        // A pending client search means this turn continues
                        // with a follow-up request; the intermediate
                        // completion must not terminate the canonical turn.
                        if matches!(inference_event, InferenceEvent::Completed(_))
                            && continuation.is_some()
                            && !state.pending_client_tool_searches.is_empty()
                        {
                            continue;
                        }
                        yield inference_event;
                    }
                }
            }

            let trailing_event = if buffer.trim().is_empty() {
                None
            } else {
                parse_sse_frame(&buffer)?
            };
            if let Some(event) = trailing_event {
                for inference_event in events_from_sse_event(&event, &mut state) {
                    if matches!(inference_event, InferenceEvent::Completed(_))
                        && continuation.is_some()
                        && !state.pending_client_tool_searches.is_empty()
                    {
                        continue;
                    }
                    yield inference_event;
                }
            }

            if !state.terminal {
                Err(anyhow::anyhow!("stream closed before response.completed"))?;
            }

            let pending = std::mem::take(&mut state.pending_client_tool_searches);
            let Some(ctx) = continuation.as_mut() else {
                break;
            };
            if pending.is_empty() {
                break;
            }

            // Execute the searches locally against the runtime catalog and
            // continue the turn with tool_search_output items. Execution of
            // any selected tool still flows through TurnToolExecutor.
            for item in pending {
                let call_id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let query = item
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let hits = ctx.catalog.search(&query, 10);
                let selected: Vec<Value> = hits
                    .iter()
                    .map(|hit| Value::String(hit.name.clone()))
                    .collect();
                let output_tools: Vec<Value> = hits
                    .iter()
                    .map(|hit| {
                        json!({
                            "id": hit.id,
                            "name": hit.name,
                            "description": hit.description,
                        })
                    })
                    .collect();
                for event in emit_hosted_tool_completed_events(
                    HostedToolCallCompleted {
                        id: item
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or(&call_id)
                            .to_string(),
                        name: "tool_search".to_string(),
                        arguments: json!({
                            "query": query,
                            "selected_tools": selected,
                            "executor": "client",
                        })
                        .to_string(),
                    },
                    &mut state,
                ) {
                    yield event;
                }
                if let Some(input) = ctx.body.get_mut("input").and_then(Value::as_array_mut) {
                    input.push(item.clone());
                    input.push(json!({
                        "type": "tool_search_output",
                        "call_id": call_id,
                        "output": Value::Array(output_tools).to_string(),
                    }));
                }
            }

            let retried = send_responses_request(
                &ctx.base_url,
                &ctx.api_key,
                &ctx.headers,
                ctx.grok_conversation_id.as_deref(),
                &ctx.body,
                ctx.policy.as_ref(),
            )
            .await?;
            for retry_event in retried.retry_events {
                yield InferenceEvent::ProviderMetadata(retry_event);
            }
            response = retried.response;
        }
    })
}

#[derive(Default)]
struct ResponsesStreamState {
    terminal: bool,
    streamed_final_text: bool,
    current_message_phase: String,
    message_phases: HashMap<String, String>,
    streamed_message_ids: HashSet<String>,
    tool_arguments: HashMap<String, String>,
    tool_names: HashMap<String, String>,
    tool_call_ids: HashMap<String, String>,
    tool_name_map: HashMap<String, String>,
    emitted_tool_call_ids: HashSet<String>,
    emitted_hosted_tool_start_ids: HashSet<String>,
    emitted_hosted_tool_complete_ids: HashSet<String>,
    reasoning_delta_keys: HashSet<String>,
    /// Completed `tool_search_call` items without provider-side results:
    /// the client must execute the search and continue the turn.
    pending_client_tool_searches: Vec<Value>,
}

/**
 * A `tool_search_call` the client must execute: it carries a query but no
 * provider-side results. Hosted (server-executed) searches always include
 * `results`; failed calls report a failed status instead.
 */
fn is_client_executed_tool_search(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("tool_search_call")
        && item.get("results").is_none()
        && item
            .get("query")
            .or_else(|| item.get("queries"))
            .is_some()
        && item.get("status").and_then(Value::as_str) != Some("failed")
}

#[derive(Debug, PartialEq)]
struct SseEvent {
    event: Option<String>,
    data: Value,
}

fn take_sse_frame(buffer: &str) -> Option<(String, usize)> {
    let lf = buffer.find("\n\n").map(|idx| (idx, 2));
    let crlf = buffer.find("\r\n\r\n").map(|idx| (idx, 4));
    let (idx, delimiter_len) = match (lf, crlf) {
        (Some(lf), Some(crlf)) => lf.min(crlf),
        (Some(lf), None) => lf,
        (None, Some(crlf)) => crlf,
        (None, None) => return None,
    };
    Some((buffer[..idx].to_string(), idx + delimiter_len))
}

fn parse_sse_frame(frame: &str) -> anyhow::Result<Option<SseEvent>> {
    let mut event = None;
    let mut data = Vec::new();

    for raw_line in frame.lines() {
        let line = raw_line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim_start().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start());
        }
    }

    if data.is_empty() {
        return Ok(None);
    }

    let data = data.join("\n");
    if data.trim() == "[DONE]" {
        return Ok(None);
    }

    Ok(Some(SseEvent {
        event,
        data: serde_json::from_str(&data).map_err(|err| {
            anyhow::anyhow!(
                "failed to parse Responses SSE data as JSON: {err}; data: {}",
                error_body_excerpt(&data)
            )
        })?,
    }))
}

fn events_from_sse_event(
    event: &SseEvent,
    state: &mut ResponsesStreamState,
) -> Vec<InferenceEvent> {
    let kind = event
        .data
        .get("type")
        .and_then(|value| value.as_str())
        .or(event.event.as_deref())
        .unwrap_or_default();

    match kind {
        "response.output_text.delta" => {
            let phase = output_text_phase(&event.data, state);
            if let Some(item_id) = event.data.get("item_id").and_then(Value::as_str) {
                state.streamed_message_ids.insert(item_id.to_string());
            }
            event
                .data
                .get("delta")
                .and_then(|value| value.as_str())
                .map(|text| {
                    if is_final_answer_phase(&phase) {
                        state.streamed_final_text = true;
                    }
                    InferenceEvent::MessageDelta(MessageDelta {
                        text: text.to_string(),
                        phase: (!phase.is_empty()).then_some(phase),
                    })
                })
                .into_iter()
                .collect()
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            if let Some(key) = reasoning_content_key(kind, &event.data) {
                state.reasoning_delta_keys.insert(key);
            }
            event
                .data
                .get("delta")
                .and_then(|value| value.as_str())
                .map(|text| {
                    InferenceEvent::ReasoningDelta(ReasoningDelta {
                        text: text.to_string(),
                    })
                })
                .into_iter()
                .collect()
        }
        "response.reasoning_summary_text.done" | "response.reasoning_text.done" => {
            let key = reasoning_content_key(kind, &event.data);
            if key
                .as_ref()
                .is_some_and(|key| state.reasoning_delta_keys.contains(key))
            {
                return Vec::new();
            }
            event
                .data
                .get("text")
                .and_then(|value| value.as_str())
                .map(|text| {
                    InferenceEvent::ReasoningDelta(ReasoningDelta {
                        text: text.to_string(),
                    })
                })
                .into_iter()
                .collect()
        }
        "response.output_item.added" => {
            if let Some(item) = event.data.get("item") {
                record_output_item(item, state);
                if is_compaction_item(item) {
                    return vec![compaction_event(item, "started")];
                }
                if let Some(call) = hosted_tool_call_started_from_item(item) {
                    return emit_hosted_tool_start_once(call, state)
                        .into_iter()
                        .collect();
                }
                if let Some(call) = started_function_call(item, state) {
                    return vec![InferenceEvent::ToolCallStarted(call)];
                }
            }
            Vec::new()
        }
        "response.web_search_call.searching" => event
            .data
            .get("item_id")
            .and_then(Value::as_str)
            .map(|id| HostedToolCallStarted {
                id: id.to_string(),
                name: "web_search".to_string(),
            })
            .and_then(|call| emit_hosted_tool_start_once(call, state))
            .into_iter()
            .collect(),
        "response.function_call_arguments.delta" => {
            if let Some(item_id) = event.data.get("item_id").and_then(Value::as_str)
                && let Some(delta) = event.data.get("delta").and_then(Value::as_str)
            {
                state
                    .tool_arguments
                    .entry(item_id.to_string())
                    .or_default()
                    .push_str(delta);
                let id = state
                    .tool_call_ids
                    .get(item_id)
                    .cloned()
                    .unwrap_or_else(|| item_id.to_string());
                return vec![InferenceEvent::ToolCallDelta(ToolCallDelta {
                    id,
                    arguments_delta: delta.to_string(),
                })];
            }
            Vec::new()
        }
        "response.function_call_arguments.done" => finalized_function_call(&event.data, state)
            .and_then(|call| emit_tool_call_once(call, state))
            .into_iter()
            .collect(),
        "response.output_item.done" => {
            let mut events = Vec::new();
            if let Some(item) = event.data.get("item") {
                record_output_item(item, state);
                if is_compaction_item(item) {
                    events.push(compaction_event(item, "completed"));
                }
                if let Some(message) = message_delta_from_done_item(item, state) {
                    events.push(message);
                }
                if is_client_executed_tool_search(item) {
                    // Completion is emitted after the local search runs.
                    state.pending_client_tool_searches.push(item.clone());
                    if let Some(started) = hosted_tool_call_started_from_item(item)
                        && let Some(event) = emit_hosted_tool_start_once(started, state)
                    {
                        events.push(event);
                    }
                } else if let Some(call) = hosted_tool_call_completed_from_item(item) {
                    events.extend(emit_hosted_tool_completed_events(call, state));
                }
                events.extend(
                    extract_tool_calls_from_item(item, &state.tool_name_map)
                        .into_iter()
                        .filter_map(|call| emit_tool_call_once(call, state)),
                );
            }
            events
        }
        "response.completed" => {
            state.terminal = true;
            let response = event.data.get("response").unwrap_or(&event.data);
            let mut events = Vec::new();
            events.extend(message_deltas_from_response(response, state));
            // Recover unstreamed client-executed searches before treating
            // any tool_search_call as a hosted completion.
            if let Some(output) = response.get("output").and_then(Value::as_array) {
                for item in output {
                    if is_client_executed_tool_search(item)
                        && !state
                            .pending_client_tool_searches
                            .iter()
                            .any(|pending| pending.get("id") == item.get("id"))
                    {
                        state.pending_client_tool_searches.push(item.clone());
                        if let Some(started) = hosted_tool_call_started_from_item(item)
                            && let Some(event) = emit_hosted_tool_start_once(started, state)
                        {
                            events.push(event);
                        }
                    }
                }
            }
            for call in extract_hosted_tool_calls(response) {
                events.extend(emit_hosted_tool_completed_events(call, state));
            }
            for call in extract_tool_calls(response, &state.tool_name_map) {
                if let Some(call) = emit_tool_call_once(call, state) {
                    events.push(call);
                }
            }
            if let Some(usage) = extract_usage(response) {
                events.push(InferenceEvent::Usage(usage));
            }
            events.push(InferenceEvent::ProviderMetadata(response.clone()));
            events.push(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: response
                    .get("status")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                provider_response_id: response
                    .get("id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            }));
            events
        }
        "response.failed" | "response.incomplete" | "error" => {
            state.terminal = true;
            vec![InferenceEvent::Failed(InferenceFailure {
                message: stream_error_message(&event.data, kind),
            })]
        }
        _ => Vec::new(),
    }
}

fn reasoning_content_key(kind: &str, data: &Value) -> Option<String> {
    let item_id = data.get("item_id").and_then(Value::as_str)?;
    let content_index = data
        .get("content_index")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let kind = kind
        .strip_suffix(".delta")
        .or_else(|| kind.strip_suffix(".done"))
        .unwrap_or(kind);
    Some(format!("{kind}:{item_id}:{content_index}"))
}

fn output_text_phase(data: &Value, state: &ResponsesStreamState) -> String {
    data.get("item_id")
        .and_then(Value::as_str)
        .and_then(|item_id| state.message_phases.get(item_id))
        .cloned()
        .unwrap_or_else(|| state.current_message_phase.clone())
}

fn is_final_answer_phase(phase: &str) -> bool {
    phase.is_empty() || phase == FINAL_ANSWER_PHASE
}

fn hosted_tool_call_started_from_item(item: &Value) -> Option<HostedToolCallStarted> {
    let name = hosted_tool_name(item)?;
    let id = item.get("id").and_then(Value::as_str)?;
    Some(HostedToolCallStarted {
        id: id.to_string(),
        name: name.to_string(),
    })
}

fn hosted_tool_call_completed_from_item(item: &Value) -> Option<HostedToolCallCompleted> {
    let name = hosted_tool_name(item)?;
    let id = item.get("id").and_then(Value::as_str)?;
    Some(HostedToolCallCompleted {
        id: id.to_string(),
        name: name.to_string(),
        arguments: hosted_tool_arguments(item),
    })
}

fn hosted_tool_name(item: &Value) -> Option<&'static str> {
    match item.get("type").and_then(Value::as_str) {
        Some("web_search_call") => Some("web_search"),
        Some("tool_search_call") => Some("tool_search"),
        _ => None,
    }
}

fn hosted_tool_arguments(item: &Value) -> String {
    let mut arguments = serde_json::Map::new();
    if let Some(action) = item.get("action").and_then(Value::as_object) {
        if let Some(action_type) = action.get("type").and_then(Value::as_str) {
            arguments.insert("action".to_string(), Value::String(action_type.to_string()));
        }
        if let Some(query) = action
            .get("query")
            .or_else(|| action.get("pattern"))
            .and_then(Value::as_str)
        {
            arguments.insert("query".to_string(), Value::String(query.to_string()));
        } else if let Some(queries) = action.get("queries").and_then(Value::as_array)
            && let Some(query) = queries.first().and_then(Value::as_str)
        {
            arguments.insert("query".to_string(), Value::String(query.to_string()));
        }
        if let Some(url) = action.get("url").and_then(Value::as_str) {
            arguments.insert("url".to_string(), Value::String(url.to_string()));
        }
    }
    // tool_search_call items carry the search query and the searched tool
    // selection at the item level; preserve them so the canonical hosted
    // tool-call events never lose searched tool ids.
    if !arguments.contains_key("query") {
        if let Some(query) = item.get("query").and_then(Value::as_str) {
            arguments.insert("query".to_string(), Value::String(query.to_string()));
        } else if let Some(queries) = item.get("queries").and_then(Value::as_array)
            && let Some(query) = queries.first().and_then(Value::as_str)
        {
            arguments.insert("query".to_string(), Value::String(query.to_string()));
        }
    }
    if let Some(results) = item.get("results").and_then(Value::as_array) {
        let selected: Vec<Value> = results
            .iter()
            .filter_map(|result| {
                result
                    .get("name")
                    .or_else(|| result.get("tool_name"))
                    .and_then(Value::as_str)
                    .map(|name| Value::String(name.to_string()))
            })
            .collect();
        if !selected.is_empty() {
            arguments.insert("selected_tools".to_string(), Value::Array(selected));
        }
    }
    Value::Object(arguments).to_string()
}

fn emit_hosted_tool_start_once(
    call: HostedToolCallStarted,
    state: &mut ResponsesStreamState,
) -> Option<InferenceEvent> {
    state
        .emitted_hosted_tool_start_ids
        .insert(call.id.clone())
        .then_some(InferenceEvent::HostedToolCallStarted(call))
}

fn emit_hosted_tool_completed_events(
    call: HostedToolCallCompleted,
    state: &mut ResponsesStreamState,
) -> Vec<InferenceEvent> {
    let mut events = Vec::new();
    if let Some(started) = emit_hosted_tool_start_once(
        HostedToolCallStarted {
            id: call.id.clone(),
            name: call.name.clone(),
        },
        state,
    ) {
        events.push(started);
    }
    if state
        .emitted_hosted_tool_complete_ids
        .insert(call.id.clone())
    {
        events.push(InferenceEvent::HostedToolCallCompleted(call));
    }
    events
}

fn record_output_item(item: &Value, state: &mut ResponsesStreamState) {
    match item.get("type").and_then(Value::as_str) {
        Some("message") => {
            let phase = item
                .get("phase")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            state.current_message_phase = phase.clone();
            if let Some(id) = item.get("id").and_then(Value::as_str) {
                state.message_phases.insert(id.to_string(), phase);
            }
        }
        Some("function_call") => {
            let Some(id) = item.get("id").and_then(Value::as_str) else {
                return;
            };
            if let Some(name) = item.get("name").and_then(Value::as_str) {
                state.tool_names.insert(id.to_string(), name.to_string());
            }
            if let Some(call_id) = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
            {
                state
                    .tool_call_ids
                    .insert(id.to_string(), call_id.to_string());
            }
            if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
                state
                    .tool_arguments
                    .insert(id.to_string(), arguments.to_string());
            }
        }
        _ => {}
    }
}

fn message_delta_from_done_item(
    item: &Value,
    state: &mut ResponsesStreamState,
) -> Option<InferenceEvent> {
    if item.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let id = item.get("id").and_then(Value::as_str);
    if id.is_some_and(|id| state.streamed_message_ids.contains(id)) {
        return None;
    }
    let text = output_text_from_message_item(item)?;
    let phase = item
        .get("phase")
        .and_then(Value::as_str)
        .unwrap_or(FINAL_ANSWER_PHASE)
        .to_string();
    if is_final_answer_phase(&phase) {
        state.streamed_final_text = true;
    }
    if let Some(id) = id {
        state.streamed_message_ids.insert(id.to_string());
    }
    Some(InferenceEvent::MessageDelta(MessageDelta {
        text,
        phase: Some(phase),
    }))
}

fn started_function_call(item: &Value, state: &ResponsesStreamState) -> Option<ToolCallStarted> {
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    let item_id = item.get("id").and_then(Value::as_str)?;
    let id = state
        .tool_call_ids
        .get(item_id)
        .cloned()
        .unwrap_or_else(|| item_id.to_string());
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .map(|name| map_tool_name(name, &state.tool_name_map).to_string())
        .or_else(|| {
            state
                .tool_names
                .get(item_id)
                .map(|name| map_tool_name(name, &state.tool_name_map).to_string())
        })?;
    Some(ToolCallStarted { id, name })
}

fn finalized_function_call(
    data: &Value,
    state: &mut ResponsesStreamState,
) -> Option<ToolCallCompleted> {
    let item_id = data.get("item_id").and_then(Value::as_str)?;
    let arguments = data
        .get("arguments")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            state
                .tool_arguments
                .get(item_id)
                .cloned()
                .unwrap_or_else(|| "{}".to_string())
        });
    state
        .tool_arguments
        .insert(item_id.to_string(), arguments.clone());
    let name = data
        .get("name")
        .and_then(Value::as_str)
        .map(|name| map_tool_name(name, &state.tool_name_map).to_string())
        .or_else(|| {
            state
                .tool_names
                .get(item_id)
                .map(|name| map_tool_name(name, &state.tool_name_map).to_string())
        })?;
    let id = state
        .tool_call_ids
        .get(item_id)
        .cloned()
        .unwrap_or_else(|| item_id.to_string());

    Some(ToolCallCompleted {
        id,
        name,
        arguments,
    })
}

fn emit_tool_call_once(
    call: ToolCallCompleted,
    state: &mut ResponsesStreamState,
) -> Option<InferenceEvent> {
    state
        .emitted_tool_call_ids
        .insert(call.id.clone())
        .then_some(InferenceEvent::ToolCallCompleted(call))
}

fn extract_tool_calls_from_item(
    item: &Value,
    tool_name_map: &HashMap<String, String>,
) -> Vec<ToolCallCompleted> {
    if item.get("type").and_then(|value| value.as_str()) != Some("function_call") {
        return Vec::new();
    }

    let Some(id) = item
        .get("call_id")
        .or_else(|| item.get("id"))
        .and_then(|value| value.as_str())
    else {
        return Vec::new();
    };
    let Some(name) = item.get("name").and_then(|value| value.as_str()) else {
        return Vec::new();
    };
    vec![ToolCallCompleted {
        id: id.to_string(),
        name: map_tool_name(name, tool_name_map).to_string(),
        arguments: item
            .get("arguments")
            .and_then(|value| value.as_str())
            .unwrap_or("{}")
            .to_string(),
    }]
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

fn response_input_items(
    request: &AgentInferenceRequest,
    tool_name_map: &ResponsesToolNameMap,
) -> Vec<Value> {
    let mut items = Vec::new();
    let mut provider_output_call_ids = HashSet::new();
    let completed_tool_call_ids = completed_tool_call_ids(&request.transcript);
    let known_tool_call_ids = known_tool_call_ids(&request.transcript);

    for conversation_item in &request.transcript {
        let mapped = match conversation_item {
            roder_api::transcript::TranscriptItem::UserMessage(message) => Some(json!({
                "type": "message",
                "role": "user",
                "content": user_message_content(message)
            })),
            roder_api::transcript::TranscriptItem::AssistantMessage(message) => Some(json!({
                "type": "message",
                "role": "assistant",
                "phase": message.phase.as_deref().unwrap_or(FINAL_ANSWER_PHASE),
                "content": [{ "type": "output_text", "text": message.text }]
            })),
            roder_api::transcript::TranscriptItem::ReasoningSummary(summary) => Some(json!({
                "type": "reasoning",
                "summary": [{ "type": "summary_text", "text": summary.text }]
            })),
            roder_api::transcript::TranscriptItem::ToolCall(call) => {
                if provider_output_call_ids.contains(&call.id)
                    || !completed_tool_call_ids.contains(&call.id)
                {
                    None
                } else {
                    let item_id = fallback_function_call_item_id(&call.id);
                    let name = tool_name_map.replay_api_name(&call.name);
                    Some(json!({
                        "type": "function_call",
                        "id": item_id,
                        "call_id": call.id,
                        "name": name,
                        "arguments": call.arguments,
                        "status": "completed"
                    }))
                }
            }
            roder_api::transcript::TranscriptItem::ToolResult(result) => {
                if known_tool_call_ids.contains(&result.id) {
                    Some(json!({
                        "type": "function_call_output",
                        "call_id": result.id,
                        "output": result.result
                    }))
                } else {
                    None
                }
            }
            roder_api::transcript::TranscriptItem::ContextCompaction(compaction) => Some(json!({
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": format!("Context summary:\n{}", compaction.summary) }]
            })),
            roder_api::transcript::TranscriptItem::ProviderMetadata(metadata) => {
                append_provider_output_items(
                    metadata,
                    &mut items,
                    &mut provider_output_call_ids,
                    &completed_tool_call_ids,
                    tool_name_map,
                );
                None
            }
            _ => None,
        };
        if let Some(item) = mapped {
            items.push(item);
        }
    }

    items
}

fn known_tool_call_ids(transcript: &[roder_api::transcript::TranscriptItem]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for item in transcript {
        match item {
            roder_api::transcript::TranscriptItem::ToolCall(call) => {
                ids.insert(call.id.clone());
            }
            roder_api::transcript::TranscriptItem::ProviderMetadata(metadata) => {
                if let Some(output) = metadata.get("output").and_then(Value::as_array) {
                    ids.extend(output.iter().filter_map(|item| {
                        (item.get("type").and_then(Value::as_str) == Some("function_call"))
                            .then(|| {
                                item.get("call_id")
                                    .and_then(Value::as_str)
                                    .map(str::to_string)
                            })
                            .flatten()
                    }));
                }
            }
            _ => {}
        }
    }
    ids
}

fn completed_tool_call_ids(
    transcript: &[roder_api::transcript::TranscriptItem],
) -> HashSet<String> {
    transcript
        .iter()
        .filter_map(|item| match item {
            roder_api::transcript::TranscriptItem::ToolResult(result) => Some(result.id.clone()),
            _ => None,
        })
        .collect()
}

fn response_input_items_with_options(
    request: &AgentInferenceRequest,
    tool_name_map: &ResponsesToolNameMap,
    profile: ResponsesProviderProfile,
) -> Vec<Value> {
    let mut items = response_input_items(request, tool_name_map);
    if profile == ResponsesProviderProfile::OpenRouter {
        let mut instruction_items = Vec::new();
        if let Some(system) = request
            .instructions
            .system
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            instruction_items.push(system_input_message(system));
        }
        if let Some(developer) = request
            .instructions
            .developer
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            instruction_items.push(system_input_message(&format!(
                "Developer instructions:\n{developer}"
            )));
        }
        if !instruction_items.is_empty() {
            instruction_items.extend(items);
            items = instruction_items;
        }
    }
    items
}

fn system_input_message(text: &str) -> Value {
    json!({
        "type": "message",
        "role": "system",
        "content": [{ "type": "input_text", "text": text }]
    })
}

fn user_message_content(message: &roder_api::transcript::UserMessage) -> Vec<Value> {
    let mut content = Vec::new();
    if !message.text.is_empty() {
        content.push(json!({ "type": "input_text", "text": message.text }));
    }
    content.extend(message.images.iter().map(|image| {
        json!({
            "type": "input_image",
            "image_url": image.image_url,
        })
    }));
    if content.is_empty() {
        content.push(json!({ "type": "input_text", "text": "" }));
    }
    content
}

fn fallback_function_call_item_id(call_id: &str) -> String {
    if call_id.starts_with("fc_") {
        call_id.to_string()
    } else if let Some(suffix) = call_id.strip_prefix("call_") {
        format!("fc_{suffix}")
    } else {
        format!("fc_{call_id}")
    }
}

fn append_provider_output_items(
    metadata: &Value,
    items: &mut Vec<Value>,
    provider_output_call_ids: &mut HashSet<String>,
    completed_tool_call_ids: &HashSet<String>,
    tool_name_map: &ResponsesToolNameMap,
) {
    let Some(output) = metadata.get("output").and_then(Value::as_array) else {
        return;
    };
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") => {
                let call_id = item.get("call_id").and_then(Value::as_str);
                let Some(call_id) = call_id else {
                    continue;
                };
                if !completed_tool_call_ids.contains(call_id) {
                    continue;
                }
                provider_output_call_ids.insert(call_id.to_string());
                let mut item = item.clone();
                if let Some(name) = item.get("name").and_then(Value::as_str) {
                    item["name"] = json!(tool_name_map.replay_api_name(name));
                }
                items.push(item);
            }
            Some("reasoning") => items.push(item.clone()),
            Some(kind) if is_compaction_type(kind) => items.push(item.clone()),
            _ => {}
        }
    }
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
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
        .filter_map(|item| {
            let id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_str())?
                .to_string();
            let name = map_tool_name(item.get("name").and_then(|v| v.as_str())?, tool_name_map)
                .to_string();
            let arguments = item
                .get("arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}")
                .to_string();
            Some(ToolCallCompleted {
                id,
                name,
                arguments,
            })
        })
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
        .with_cached_prompt_tokens(cached_prompt_tokens),
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

    fn request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: "openai".to_string(),
                model: "gpt-5.5".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("be helpful".to_string()),
                developer: None,
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
        let (_, tool_name_map) = responses_tools(request);
        response_input_items(request, &tool_name_map)
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
        assert_eq!(body["tools"][1], json!({ "type": "tool_search" }));
        assert_eq!(body["tool_choice"], "auto");
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
            r#"{"data":[{"id":"x-ai/grok-build-0.1","name":"Grok Build 0.1","context_length":256000}]}"#,
        )])
        .await;

        let models = discover_models(&base_url, Some("secret")).await.unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "x-ai/grok-build-0.1");
        assert_eq!(models[0].name, "Grok Build 0.1");
        assert_eq!(models[0].context_window, Some(256_000));
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

        const FIRST_SSE: &str = "data: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"ts_9\",\"type\":\"tool_search_call\",\"status\":\"completed\",\"query\":\"read files\"}}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"output\":[{\"id\":\"ts_9\",\"type\":\"tool_search_call\",\"status\":\"completed\",\"query\":\"read files\"}]}}\n\n";
        const SECOND_SSE: &str = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_2\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"done\"}]}]}}\n\n";

        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let base_url = spawn_recording_server(
            vec![("/responses", 200, FIRST_SSE), ("/responses", 200, SECOND_SSE)],
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
            Some(ClientToolSearchContext {
                base_url: base_url.clone(),
                api_key: "secret".to_string(),
                headers: Vec::new(),
                grok_conversation_id: None,
                body: body.clone(),
                policy: None,
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
        assert_eq!(completions[0].provider_response_id.as_deref(), Some("resp_2"));

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
        assert_eq!(output["call_id"], "ts_9");
        assert!(output["output"].as_str().unwrap().contains("read_file"));
        assert!(!output["output"].as_str().unwrap().contains("deploy_app"));
    }

    #[tokio::test]
    async fn retry_recovers_by_removing_missing_function_call_output() {
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

        let response = send_responses_request(
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
        .unwrap();

        assert!(response.response.status().is_success());
        assert_eq!(
            response.retry_events[0]["cause"],
            "missing_function_call_output_call_id"
        );
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2);
        assert!(bodies[0].contains("call_missing"));
        assert!(!bodies[1].contains("call_missing"));
        assert!(bodies[1].contains("call_ok"));
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

        let mut stream = stream_responses_sse(response.response, HashMap::new(), Vec::new());
        let next = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("silent responses stream should surface an idle timeout");

        assert!(
            next.expect("stream should yield the timeout error")
                .is_err(),
            "silent responses stream should yield an error"
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
    fn maps_parallel_tool_call_override_for_responses_requests() {
        let mut request = request();
        request.runtime.parallel_tool_calls = Some(false);

        let body = OpenAiResponsesEngine::map_request(&request);

        assert_eq!(body["parallel_tool_calls"], false);
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
        assert_eq!(body["tools"][0]["parameters"]["required"][0], "patch");
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
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "apply_patch");
        assert_eq!(tools[0]["parameters"]["required"][0], "patch");
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
        request.model.model = "x-ai/grok-build-0.1".to_string();
        request.runtime.prompt_cache_key = Some("cache-key".to_string());

        let (body, _) = OpenAiResponsesEngine::map_request_with_options(
            &request,
            RequestMappingOptions {
                profile: ResponsesProviderProfile::OpenRouter,
                thread_id: Some("thread-123"),
            },
        );

        assert_eq!(body["model"], "x-ai/grok-build-0.1");
        assert_eq!(body["reasoning"], json!({ "effort": "medium" }));
        assert_eq!(body["prompt_cache_key"], "cache-key");
        assert!(body.get("instructions").is_none());
        assert_eq!(body["input"][0]["role"], "system");
        assert_eq!(body["input"][0]["content"][0]["text"], "be helpful");
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
            vec![InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "call_1".to_string(),
                name: "memory.save".to_string(),
                arguments: "{\"entry\":\"x\"}".to_string(),
            })]
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
            vec![InferenceEvent::MessageDelta(MessageDelta {
                text: "I’ll inspect the logs first.".to_string(),
                phase: Some("commentary".to_string()),
            })]
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
            vec![InferenceEvent::MessageDelta(MessageDelta {
                text: "I’ll inspect the logs and then summarize root cause.".to_string(),
                phase: Some("commentary".to_string()),
            })]
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
    fn emits_tool_call_from_function_arguments_done() {
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
        assert_eq!(
            events_from_sse_event(&done, &mut state),
            vec![InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "call_1".to_string(),
                name: "echo".to_string(),
                arguments: "{\"text\":\"hello\"}".to_string(),
            })]
        );
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
            vec![InferenceEvent::HostedToolCallCompleted(
                HostedToolCallCompleted {
                    id: "ws_1".to_string(),
                    name: "web_search".to_string(),
                    arguments: r#"{"action":"search","query":"pandelis zembashis"}"#.to_string(),
                }
            )]
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
        assert_eq!(
            events_from_sse_event(&added, &mut state),
            vec![InferenceEvent::Compaction(CompactionProgress {
                status: "started".to_string(),
                item_id: Some("cmp_1".to_string()),
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
