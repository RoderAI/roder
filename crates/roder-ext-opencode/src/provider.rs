use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use roder_api::catalog::{PROVIDER_OPENCODE, PROVIDER_OPENCODE_GO, models_for_provider};
use roder_api::conversation::ConversationItem;
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceProviderMetadata,
    InferenceTurnContext, MessageDelta, ModelDescriptor, ProviderAuthType, TokenUsage,
    ToolCallCompleted,
};
use roder_api::reliability::{
    ReliabilityRequestPolicy, provider_retry_delay_ms, provider_retry_metadata,
    provider_retry_status_cause,
};
use roder_api::tools::ToolChoice;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
            reasoning_summaries: false,
            structured_output: true,
            image_input: false,
            prompt_cache: false,
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

        if let Some(entry) = cached
            && !entry.models.is_empty()
        {
            return Ok(entry.models);
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
        stream_chat_completions(
            self.spec.name,
            &self.base_url(),
            &api_key,
            self.request_headers(&ctx),
            request,
        )
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

async fn stream_chat_completions(
    provider_name: &str,
    base_url: &str,
    api_key: &str,
    headers: Vec<(String, String)>,
    request: AgentInferenceRequest,
) -> anyhow::Result<InferenceEventStream> {
    let (tools, tool_name_map) = chat_tools(&request);
    let mut body = json!({
        "model": request.model.model,
        "messages": chat_messages(&request, &tool_name_map),
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
        body["tool_choice"] = chat_tool_choice(&request.tool_choice, &tool_name_map);
        body["parallel_tool_calls"] = json!(request.runtime.parallel_tool_calls.unwrap_or(true));
    }
    if let Some(max_tokens) = request.output.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temperature) = request.output.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(top_p) = request.output.top_p {
        body["top_p"] = json!(top_p);
    }
    if let Some(response_format) = request.output.response_format {
        body["response_format"] = response_format;
    }

    let client = reqwest::Client::new();
    let mut http = client
        .post(format!(
            "{}/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .json(&body);
    for (key, value) in headers {
        http = http.header(key, value);
    }
    let response =
        send_chat_completion_request(provider_name, http, request.runtime.reliability.as_ref())
            .await?;

    let retry_events = response.retry_events;
    let mut bytes = response.response.bytes_stream();
    let stream = async_stream::try_stream! {
        for retry_event in retry_events {
            yield InferenceEvent::ProviderMetadata(retry_event);
        }
        let mut state = ChatStreamState::new(tool_name_map);
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk?;
            for event in state.push_chunk(&chunk)? {
                yield event;
            }
        }
        for event in state.finish() {
            yield event;
        }
    };

    Ok(Box::pin(stream))
}

struct RetriedResponse {
    response: reqwest::Response,
    retry_events: Vec<Value>,
}

async fn send_chat_completion_request(
    provider_name: &str,
    request: reqwest::RequestBuilder,
    policy: Option<&ReliabilityRequestPolicy>,
) -> anyhow::Result<RetriedResponse> {
    let policy = policy.cloned().unwrap_or_default();
    let attempts = policy.provider_retry_max_attempts.max(1);
    let mut last_error = None;
    let mut retry_events = Vec::new();
    for attempt in 1..=attempts {
        let Some(request) = request.try_clone() else {
            return request
                .send()
                .await
                .map(|response| RetriedResponse {
                    response,
                    retry_events,
                })
                .map_err(Into::into);
        };
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                return Ok(RetriedResponse {
                    response,
                    retry_events,
                });
            }
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                let retryable = policy
                    .provider_retry_status_codes
                    .contains(&status.as_u16());
                last_error = Some(format!(
                    "{provider_name} Chat Completions error {status}: {text}"
                ));
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
            Err(err) => {
                last_error = Some(err.to_string());
                if attempt < attempts {
                    push_retry_event(&mut retry_events, attempt, "transport_error", &policy);
                    retry_sleep(&policy, attempt).await;
                    continue;
                }
            }
        }
        break;
    }
    anyhow::bail!(last_error.unwrap_or_else(|| format!("{provider_name} request failed")))
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

#[derive(Debug, Default)]
struct ChatToolNameMap {
    tool_name_to_api_name: HashMap<String, String>,
    api_name_to_tool_name: HashMap<String, String>,
}

impl ChatToolNameMap {
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

    fn canonical_name<'a>(&'a self, api_name: &'a str) -> &'a str {
        self.api_name_to_tool_name
            .get(api_name)
            .map_or(api_name, String::as_str)
    }
}

fn chat_tools(request: &AgentInferenceRequest) -> (Vec<Value>, ChatToolNameMap) {
    let mut tools = Vec::new();
    let mut used_tool_names = HashSet::new();
    let mut tool_name_map = ChatToolNameMap::default();
    for tool in &request.tools {
        let tool = tool.normalized_for_model(roder_api::ToolSchemaPolicy::warning());
        let api_name = chat_tool_name(&tool.name, &mut used_tool_names);
        tool_name_map.register(&tool.name, &api_name);
        tools.push(json!({
            "type": "function",
            "function": {
                "name": api_name,
                "description": tool.description,
                "parameters": tool.parameters,
            },
        }));
    }
    (tools, tool_name_map)
}

fn chat_tool_name(tool_name: &str, used_tool_names: &mut HashSet<String>) -> String {
    let base_name = tool_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
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

fn chat_tool_choice(choice: &ToolChoice, tool_name_map: &ChatToolNameMap) -> Value {
    match choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::Any => json!("required"),
        ToolChoice::None => json!("none"),
        ToolChoice::Specific(name) => json!({
            "type": "function",
            "function": { "name": tool_name_map.api_name(name) },
        }),
    }
}

fn chat_messages(request: &AgentInferenceRequest, tool_name_map: &ChatToolNameMap) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(system) = &request.instructions.system {
        messages.push(json!({ "role": "system", "content": system }));
    }
    if let Some(developer) = &request.instructions.developer {
        messages.push(json!({ "role": "system", "content": developer }));
    }
    for item in &request.conversation {
        match item {
            ConversationItem::UserMessage(message) => {
                messages.push(json!({ "role": "user", "content": message.text }));
            }
            ConversationItem::AssistantMessage(message) => {
                if !message.text.is_empty() {
                    messages.push(json!({ "role": "assistant", "content": message.text }));
                }
            }
            ConversationItem::ToolCall(call) => {
                messages.push(json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": tool_name_map.api_name(&call.name),
                            "arguments": call.arguments,
                        },
                    }],
                }));
            }
            ConversationItem::ToolResult(result) => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": result.id,
                    "content": result.result,
                }));
            }
            ConversationItem::ContextCompaction(compaction) => {
                messages.push(json!({ "role": "system", "content": compaction.summary }));
            }
            ConversationItem::ReasoningSummary(summary) => {
                messages.push(json!({ "role": "assistant", "content": summary.text }));
            }
            ConversationItem::FileChange(_)
            | ConversationItem::Error(_)
            | ConversationItem::ProviderMetadata(_) => {}
        }
    }
    messages
}

#[derive(Debug)]
struct ChatStreamState {
    buffer: String,
    tool_name_map: ChatToolNameMap,
    tool_calls: BTreeMap<u64, PartialChatToolCall>,
    stop_reason: Option<String>,
    provider_response_id: Option<String>,
    usage: Option<TokenUsage>,
    metadata_chunks: Vec<Value>,
}

#[derive(Debug, Default)]
struct PartialChatToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl ChatStreamState {
    fn new(tool_name_map: ChatToolNameMap) -> Self {
        Self {
            buffer: String::new(),
            tool_name_map,
            tool_calls: BTreeMap::new(),
            stop_reason: None,
            provider_response_id: None,
            usage: None,
            metadata_chunks: Vec::new(),
        }
    }

    fn push_chunk(&mut self, chunk: &[u8]) -> anyhow::Result<Vec<InferenceEvent>> {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        let mut events = Vec::new();
        while let Some((pos, separator_len)) = sse_frame_boundary(&self.buffer) {
            let frame = self.buffer[..pos].to_string();
            self.buffer.drain(..pos + separator_len);
            events.extend(self.push_sse_frame(&frame)?);
        }
        Ok(events)
    }

    fn push_sse_frame(&mut self, frame: &str) -> anyhow::Result<Vec<InferenceEvent>> {
        let mut events = Vec::new();
        let data = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() || data == "[DONE]" {
            return Ok(events);
        }

        let value: Value = serde_json::from_str(&data)?;
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            self.provider_response_id = Some(id.to_string());
        }
        if let Some(usage) = extract_chat_usage(&value) {
            self.usage = Some(usage);
        }
        self.metadata_chunks.push(value.clone());

        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return Ok(events);
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.stop_reason = Some(reason.to_string());
        }
        let Some(delta) = choice.get("delta") else {
            return Ok(events);
        };
        if let Some(content) = delta.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
            events.push(InferenceEvent::MessageDelta(MessageDelta {
                text: content.to_string(),
                phase: None,
            }));
        }
        for tool_call in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let index = tool_call.get("index").and_then(Value::as_u64).unwrap_or(0);
            let partial = self.tool_calls.entry(index).or_default();
            if let Some(id) = tool_call.get("id").and_then(Value::as_str) {
                partial.id = Some(id.to_string());
            }
            if let Some(function) = tool_call.get("function") {
                if let Some(name) = function.get("name").and_then(Value::as_str) {
                    partial.name = Some(name.to_string());
                }
                if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                    partial.arguments.push_str(arguments);
                }
            }
        }
        Ok(events)
    }

    fn finish(self) -> Vec<InferenceEvent> {
        let mut events = Vec::new();
        for (_, call) in self.tool_calls {
            let Some(id) = call.id else {
                continue;
            };
            let Some(name) = call.name else {
                continue;
            };
            events.push(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id,
                name: self.tool_name_map.canonical_name(&name).to_string(),
                arguments: call.arguments,
            }));
        }
        if let Some(usage) = self.usage {
            events.push(InferenceEvent::Usage(usage));
        }
        events.push(InferenceEvent::ProviderMetadata(json!({
            "chunks": self.metadata_chunks,
        })));
        events.push(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: self.stop_reason,
            provider_response_id: self.provider_response_id,
        }));
        events
    }
}

fn extract_chat_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    Some(TokenUsage {
        prompt_tokens: number_to_u32(usage.get("prompt_tokens")).unwrap_or_default(),
        completion_tokens: number_to_u32(usage.get("completion_tokens")).unwrap_or_default(),
        total_tokens: number_to_u32(usage.get("total_tokens")).unwrap_or_default(),
    })
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|n| u32::try_from(n).ok())
}

fn sse_frame_boundary(buffer: &str) -> Option<(usize, usize)> {
    match (buffer.find("\n\n"), buffer.find("\r\n\r\n")) {
        (Some(lf), Some(crlf)) if lf < crlf => Some((lf, 2)),
        (Some(_), Some(crlf)) => Some((crlf, 4)),
        (Some(lf), None) => Some((lf, 2)),
        (None, Some(crlf)) => Some((crlf, 4)),
        (None, None) => None,
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

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use roder_api::conversation::{ConversationItem, UserMessage};
    use roder_api::inference::InferenceEngine;
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };
    use roder_api::reliability::ReliabilityRequestPolicy;
    use roder_api::tools::{ToolChoice, ToolSpec};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

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

    #[tokio::test]
    async fn stream_turn_uses_chat_completions_for_opencode_models() {
        let server = spawn_chat_server(
            "/chat/completions",
            concat!(
                "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\r\n\r\n",
                "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\r\n\r\n",
                "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"type\":\"function\",\"function\":{\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\"}}]},\"finish_reason\":null}]}\r\n\r\n",
                "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"date\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\r\n\r\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":4,\"total_tokens\":7}}\r\n\r\n",
                "data: [DONE]\r\n\r\n",
            ),
        )
        .await;
        let engine = OpenCodeInferenceEngine::new(
            OpenCodeConfig {
                api_key: Some("secret".to_string()),
                base_url: Some(server.base_url.clone()),
                project_id: None,
            },
            OpenCodeProviderSpec::zen(),
        );
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: PROVIDER_OPENCODE.to_string(),
                model: "minimax-m2.5-free".to_string(),
            },
            instructions: InstructionBundle::default(),
            conversation: vec![ConversationItem::UserMessage(UserMessage::text("hi"))],
            tools: vec![ToolSpec {
                name: "exec_command".to_string(),
                description: "Run a command".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": { "cmd": { "type": "string" } },
                    "required": ["cmd"]
                }),
            }],
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints {
                parallel_tool_calls: Some(false),
                ..RuntimeHints::default()
            },
            metadata: json!({}),
        };

        let mut stream = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread-1",
                    turn_id: "turn-1",
                },
                request,
            )
            .await
            .unwrap();
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.unwrap());
        }
        let request_body = server.request_body.await.unwrap();

        assert_eq!(request_body["model"], "minimax-m2.5-free");
        assert_eq!(request_body["stream"], true);
        assert_eq!(request_body["stream_options"]["include_usage"], true);
        assert_eq!(request_body["parallel_tool_calls"], false);
        assert!(request_body.get("tools").is_some());
        assert!(matches!(
            events.first(),
            Some(InferenceEvent::MessageDelta(delta)) if delta.text == "Hel"
        ));
        assert!(matches!(
            events.iter().find(|event| matches!(event, InferenceEvent::ToolCallCompleted(_))),
            Some(InferenceEvent::ToolCallCompleted(call))
                if call.name == "exec_command" && call.arguments == "{\"cmd\":\"date\"}"
        ));
    }

    #[tokio::test]
    async fn retry_recovers_chat_completion_after_retryable_status() {
        let base_url = spawn_retry_chat_server(vec![
            (503, r#"{"error":"busy"}"#),
            (
                200,
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\r\n\r\ndata: [DONE]\r\n\r\n",
            ),
        ])
        .await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_max_attempts: 2,
            provider_retry_initial_backoff_ms: 0,
            provider_retry_status_codes: vec![503],
            ..ReliabilityRequestPolicy::default()
        };
        let client = reqwest::Client::new();
        let request = client
            .post(format!("{base_url}/chat/completions"))
            .bearer_auth("secret")
            .json(&json!({ "model": "opencode/gpt-5.5", "messages": [] }));

        let response = send_chat_completion_request("OpenCode Zen", request, Some(&policy))
            .await
            .unwrap();

        assert!(response.response.status().is_success());
        assert_eq!(
            response.retry_events[0]["kind"],
            "reliability_retry_attempt"
        );
    }

    #[tokio::test]
    async fn profile_request_snapshot_maps_opencode_edit_tool_and_parallel_flag() {
        let server = spawn_chat_server(
            "/chat/completions",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":0,\"total_tokens\":1}}\r\n\r\ndata: [DONE]\r\n\r\n",
        )
        .await;
        let engine = OpenCodeInferenceEngine::new(
            OpenCodeConfig {
                api_key: Some("secret".to_string()),
                base_url: Some(server.base_url.clone()),
                project_id: None,
            },
            OpenCodeProviderSpec::zen(),
        );
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: PROVIDER_OPENCODE.to_string(),
                model: "minimax-m2.5-free".to_string(),
            },
            instructions: InstructionBundle {
                system: None,
                developer: Some("profile overlay".to_string()),
            },
            conversation: vec![ConversationItem::UserMessage(UserMessage::text("edit"))],
            tools: vec![ToolSpec {
                name: "edit".to_string(),
                description: "Edit a file".to_string(),
                parameters: json!({
                    "type": "object",
                    "required": ["path", "old_string", "new_string"],
                    "properties": {
                        "path": { "type": "string" },
                        "old_string": { "type": "string" },
                        "new_string": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            }],
            tool_choice: ToolChoice::Specific("edit".to_string()),
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints {
                parallel_tool_calls: Some(false),
                ..RuntimeHints::default()
            },
            metadata: json!({
                "modelProfile": {
                    "editTool": "edit",
                    "schemaPolicy": "standard_required_first",
                    "parallelToolCalls": false
                }
            }),
        };

        let mut stream = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread-profile",
                    turn_id: "turn-profile",
                },
                request,
            )
            .await
            .unwrap();
        while stream.next().await.is_some() {}
        let request_body = server.request_body.await.unwrap();

        assert_eq!(request_body["tools"][0]["function"]["name"], "edit");
        assert_eq!(request_body["tool_choice"]["function"]["name"], "edit");
        assert_eq!(request_body["parallel_tool_calls"], false);
        assert_eq!(
            request_body["tools"][0]["function"]["parameters"]["required"],
            json!(["path", "old_string", "new_string"])
        );
    }

    #[test]
    fn normalizes_tool_schema_order_for_opencode_tools() {
        let mut request = AgentInferenceRequest {
            model: ModelSelection {
                provider: PROVIDER_OPENCODE.to_string(),
                model: "minimax-m2.5-free".to_string(),
            },
            instructions: InstructionBundle::default(),
            conversation: vec![ConversationItem::UserMessage(UserMessage::text("hi"))],
            tools: vec![ToolSpec {
                name: "exec_command".to_string(),
                description: "Run a command".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": { "command": { "type": "string" } },
                    "additionalProperties": false,
                    "required": ["command"]
                }),
            }],
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        };

        let (tools, tool_name_map) = chat_tools(&request);
        let schema = serde_json::to_string(&tools[0]["function"]["parameters"]).unwrap();

        assert_eq!(tool_name_map.api_name("exec_command"), "exec_command");
        assert!(
            schema.starts_with(r#"{"type":"object","required":["command"],"properties":"#),
            "{schema}"
        );

        request.tool_choice = ToolChoice::Specific("exec_command".to_string());
        assert_eq!(
            chat_tool_choice(&request.tool_choice, &tool_name_map)["function"]["name"],
            "exec_command"
        );
    }

    struct CapturedChatServer {
        base_url: String,
        request_body: tokio::sync::oneshot::Receiver<Value>,
    }

    async fn spawn_chat_server(
        expected_path: &'static str,
        response_body: &'static str,
    ) -> CapturedChatServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0_u8; 16 * 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("");
            assert_eq!(path, expected_path);
            let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
            tx.send(serde_json::from_str(body).unwrap()).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        CapturedChatServer {
            base_url: format!("http://{addr}"),
            request_body: rx,
        }
    }

    async fn spawn_retry_chat_server(responses: Vec<(u16, &'static str)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = vec![0_u8; 16 * 1024];
                let _ = stream.read(&mut buf).await.unwrap();
                let reason = if status == 200 { "OK" } else { "Retry" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: text/event-stream\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        format!("http://{addr}")
    }
}
