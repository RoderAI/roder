use roder_api::catalog::{PROVIDER_ANTHROPIC, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext, ModelDescriptor,
    ProviderAuthType, TokenUsage, ToolSearchProviderVariant,
};
use roder_api::reliability::{
    ReliabilityRequestPolicy, provider_retry_delay_ms, provider_retry_metadata,
    provider_retry_status_cause,
};
use serde_json::{Value, json};
use std::time::Duration;

use crate::sse::stream_anthropic_sse;

const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct AnthropicEngine {
    api_key: String,
}

impl AnthropicEngine {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    fn map_request(request: &AgentInferenceRequest) -> Value {
        let mut body = json!({
            "model": request.model.model,
            "max_tokens": request.output.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            "cache_control": { "type": "ephemeral" },
            "stream": true,
            "messages": anthropic_messages(request),
        });
        let mut system = Vec::new();
        if let Some(text) = request.instructions.system.as_deref() {
            system.push(json!({ "type": "text", "text": text }));
        }
        if let Some(text) = request.instructions.developer.as_deref() {
            system.push(json!({ "type": "text", "text": text }));
        }
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if anthropic_model_accepts_sampling_params(&request.model.model) {
            if let Some(temperature) = request.output.temperature {
                body["temperature"] = json!(temperature);
            }
            if let Some(top_p) = request.output.top_p {
                body["top_p"] = json!(top_p);
            }
        }
        if request.reasoning.enabled
            && let Some(level) = request.reasoning.level.as_deref()
            && anthropic_model_supports_effort(&request.model.model)
        {
            body["output_config"] = json!({ "effort": anthropic_effort(level) });
        }
        if !request.tools.is_empty() {
            body["tools"] = json!(anthropic_tools(request));
            body["tool_choice"] = anthropic_tool_choice(&request.tool_choice);
        }
        body
    }
}

#[async_trait::async_trait]
impl InferenceEngine for AnthropicEngine {
    fn id(&self) -> InferenceEngineId {
        "anthropic".to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: false,
            reasoning_summaries: false,
            structured_output: false,
            image_input: false,
            prompt_cache: true,
            provider_metadata: true,
            tool_search: true,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Anthropic".to_string(),
            description: Some("Anthropic API key provider".to_string()),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some("API key".to_string()),
            auth_configured: Some(true),
            recommended: true,
            sort_order: 30,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_ANTHROPIC, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let body = Self::map_request(&request);
        let (response, retry_events) = send_anthropic_request(
            "https://api.anthropic.com/v1/messages",
            &self.api_key,
            &body,
            request.runtime.reliability.as_ref(),
        )
        .await?;
        Ok(stream_anthropic_sse(response, request.tools, retry_events))
    }
}

/**
 * Sends the streaming request, retrying failures that occur before any bytes
 * of the event stream are consumed (HTTP error status, transport errors).
 * Mid-stream failures are not retried here: partial deltas have already been
 * emitted to the host, so replaying the request would duplicate them. They
 * surface as stream errors from `stream_anthropic_sse` instead.
 */
async fn send_anthropic_request(
    url: &str,
    api_key: &str,
    body: &Value,
    policy: Option<&ReliabilityRequestPolicy>,
) -> anyhow::Result<(reqwest::Response, Vec<Value>)> {
    let policy = policy.cloned().unwrap_or_default();
    let attempts = policy.provider_retry_max_attempts.max(1);
    let client = reqwest::Client::new();
    let mut last_error = None;
    let mut retry_events = Vec::new();
    for attempt in 1..=attempts {
        let response = client
            .post(url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(body)
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => {
                return Ok((response, retry_events));
            }
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                let retryable = policy
                    .provider_retry_status_codes
                    .contains(&status.as_u16());
                last_error = Some(format!("Anthropic error {status}: {text}"));
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
    anyhow::bail!(last_error.unwrap_or_else(|| "Anthropic request failed".to_string()))
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

fn anthropic_messages(request: &AgentInferenceRequest) -> Vec<Value> {
    request
        .transcript
        .iter()
        .filter_map(|item| match item {
            roder_api::transcript::TranscriptItem::UserMessage(message) => Some(json!({
                "role": "user",
                "content": [{ "type": "text", "text": message.text }]
            })),
            roder_api::transcript::TranscriptItem::AssistantMessage(message) => Some(json!({
                "role": "assistant",
                "content": [{ "type": "text", "text": message.text }]
            })),
            roder_api::transcript::TranscriptItem::ToolCall(call) => Some(json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": call.id,
                    "name": anthropic_tool_name(&call.name),
                    "input": parse_json_object(&call.arguments)
                }]
            })),
            roder_api::transcript::TranscriptItem::ToolResult(result) => Some(json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": result.id,
                    "content": result.result,
                    "is_error": result.is_error
                }]
            })),
            _ => None,
        })
        .collect()
}

fn anthropic_tool_choice(choice: &roder_api::tools::ToolChoice) -> Value {
    match choice {
        roder_api::tools::ToolChoice::Auto => json!({ "type": "auto" }),
        roder_api::tools::ToolChoice::Any => json!({ "type": "any" }),
        roder_api::tools::ToolChoice::None => json!({ "type": "none" }),
        roder_api::tools::ToolChoice::Specific(name) => {
            json!({ "type": "tool", "name": anthropic_tool_name(name) })
        }
    }
}

/// Anthropic rejects tool names outside ^[a-zA-Z0-9_-]{1,128}$, but roder tool
/// names may contain dots (discovery.read, webwright.run_script). Dots become
/// "__" on the wire; canonical_tool_name reverses the mapping on response parse.
fn anthropic_tool_name(name: &str) -> String {
    name.replace('.', "__")
}

pub(crate) fn canonical_tool_name(wire_name: &str, tools: &[roder_api::tools::ToolSpec]) -> String {
    if tools.iter().any(|tool| tool.name == wire_name) {
        return wire_name.to_string();
    }
    tools
        .iter()
        .find(|tool| anthropic_tool_name(&tool.name) == wire_name)
        .map(|tool| tool.name.clone())
        .unwrap_or_else(|| wire_name.to_string())
}

fn anthropic_tools(request: &AgentInferenceRequest) -> Vec<Value> {
    let mut tools = Vec::new();
    if anthropic_provider_native_tool_search(request) {
        tools.push(json!({
            "type": anthropic_tool_search_type(request.runtime.tool_search.provider_variant),
            "name": "tool_search"
        }));
    }
    tools.extend(request.tools.iter().map(|tool| {
        let tool = tool.normalized_for_model(roder_api::ToolSchemaPolicy::warning());
        let mut value = json!({
            "name": anthropic_tool_name(&tool.name),
            "description": tool.description,
            "input_schema": tool.parameters,
        });
        if anthropic_provider_native_tool_search(request) {
            value["defer_loading"] = json!(true);
        }
        value
    }));
    tools
}

fn anthropic_provider_native_tool_search(request: &AgentInferenceRequest) -> bool {
    request.runtime.tool_search.is_provider_native_requested()
        && request.model.provider == PROVIDER_ANTHROPIC
        && anthropic_model_supports_tool_search(&request.model.model)
}

fn anthropic_model_supports_tool_search(model: &str) -> bool {
    model.starts_with("claude-sonnet-4-6")
        || model.starts_with("claude-opus-4-8")
        || model.starts_with("claude-fable")
        || model.starts_with("claude-4")
        || model.starts_with("claude-5")
}

// Fable 5 and Opus 4.7/4.8 reject sampling parameters (`temperature`,
// `top_p`, `top_k`) with a 400 — they must be omitted from the request body.
fn anthropic_model_accepts_sampling_params(model: &str) -> bool {
    !(model.starts_with("claude-fable")
        || model.starts_with("claude-opus-4-7")
        || model.starts_with("claude-opus-4-8"))
}

// Haiku-tier models reject output_config.effort with a 400; the catalog
// declares them REASONING_NONE (roder-api catalog.rs).
fn anthropic_model_supports_effort(model: &str) -> bool {
    !model.contains("haiku")
}

fn anthropic_tool_search_type(variant: ToolSearchProviderVariant) -> &'static str {
    match variant {
        ToolSearchProviderVariant::Default | ToolSearchProviderVariant::Regex => {
            "tool_search_tool_regex_20251119"
        }
        ToolSearchProviderVariant::Bm25 => "tool_search_tool_bm25_20251119",
    }
}

fn anthropic_effort(level: &str) -> &str {
    match level {
        "minimal" => "low",
        "xhigh" => "xhigh",
        level => level,
    }
}

pub(crate) fn parse_json_object(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(|value| value.is_object())
        .unwrap_or_else(|| json!({}))
}

/// `prompt_tokens` counts the full prompt: uncached input plus cache writes
/// plus cache reads, with the cache components also reported separately.
pub(crate) fn extract_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let uncached_prompt_tokens = number_to_u32(usage.get("input_tokens")).unwrap_or_default();
    let cache_creation_tokens =
        number_to_u32(usage.get("cache_creation_input_tokens")).unwrap_or_default();
    let cache_read_tokens = number_to_u32(usage.get("cache_read_input_tokens")).unwrap_or_default();
    let prompt_tokens = uncached_prompt_tokens
        .saturating_add(cache_creation_tokens)
        .saturating_add(cache_read_tokens);
    let completion_tokens = number_to_u32(usage.get("output_tokens")).unwrap_or_default();
    Some(
        TokenUsage::new(
            prompt_tokens,
            completion_tokens,
            prompt_tokens.saturating_add(completion_tokens),
        )
        .with_cached_prompt_tokens(cache_read_tokens)
        .with_cache_creation_prompt_tokens(cache_creation_tokens),
    )
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use roder_api::inference::{
        CompletionMetadata, InferenceEvent, InstructionBundle, MessageDelta, ModelSelection,
        OutputConfig, ReasoningConfig, RuntimeHints, ToolCallCompleted, ToolCallDelta,
        ToolCallStarted,
    };
    use roder_api::reliability::ReliabilityRequestPolicy;
    use roder_api::tools::{ToolChoice, ToolSpec};
    use roder_api::transcript::{
        AssistantMessage, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: "anthropic".to_string(),
                model: "claude-sonnet-4-6".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("system".to_string()),
                developer: Some("developer".to_string()),
            },
            transcript: vec![
                TranscriptItem::UserMessage(UserMessage::text("Hello")),
                TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "Hi".to_string(),
                    phase: None,
                }),
                TranscriptItem::ToolCall(ToolCallRecord {
                    id: "toolu_1".to_string(),
                    name: "shell".to_string(),
                    arguments: r#"{"cmd":"pwd"}"#.to_string(),
                }),
                TranscriptItem::ToolResult(ToolResultRecord {
                    id: "toolu_1".to_string(),
                    name: Some("shell".to_string()),
                    result: "ok".to_string(),
                    display_payload: None,
                    is_error: false,
                }),
            ],
            tools: vec![ToolSpec {
                name: "shell".to_string(),
                description: "Run a shell command".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": { "cmd": { "type": "string" } },
                    "required": ["cmd"]
                }),
            }],
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig {
                enabled: true,
                level: Some("medium".to_string()),
            },
            output: OutputConfig {
                max_tokens: Some(100),
                temperature: Some(0.3),
                top_p: Some(0.8),
                response_format: None,
            },
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        }
    }

    #[test]
    fn maps_anthropic_request() {
        let body = AnthropicEngine::map_request(&request());
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["stream"], true);
        assert_eq!(body["cache_control"], json!({ "type": "ephemeral" }));
        assert!((body["temperature"].as_f64().unwrap() - 0.3).abs() < 1e-6);
        assert!((body["top_p"].as_f64().unwrap() - 0.8).abs() < 1e-6);
        assert_eq!(body["system"][0]["text"], "system");
        assert_eq!(body["system"][1]["text"], "developer");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][1]["role"], "assistant");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][2]["content"][0]["input"]["cmd"], "pwd");
        assert_eq!(body["messages"][3]["content"][0]["type"], "tool_result");
        assert_eq!(body["tools"][0]["name"], "shell");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["tool_choice"]["type"], "auto");
        assert_eq!(body["output_config"]["effort"], "medium");
    }

    #[test]
    fn maps_fable_5_request_without_sampling_params() {
        let mut request = request();
        request.model.model = "claude-fable-5".to_string();

        let body = AnthropicEngine::map_request(&request);

        assert_eq!(body["model"], "claude-fable-5");
        // Fable 5 rejects sampling params with a 400 — they must be omitted.
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert_eq!(body["output_config"]["effort"], "medium");
    }

    #[test]
    fn fable_5_supports_provider_native_tool_search() {
        assert!(anthropic_model_supports_tool_search("claude-fable-5"));
    }

    #[test]
    fn omits_effort_for_haiku_models() {
        let mut request = request();
        request.model.model = "claude-haiku-4-5-20251001".to_string();
        request.reasoning.enabled = true;
        request.reasoning.level = Some("high".to_string());

        let body = AnthropicEngine::map_request(&request);

        // Haiku rejects output_config.effort with a 400 — it must be omitted.
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn maps_anthropic_provider_native_tool_search_body() {
        let mut request = request();
        request.runtime.tool_search = roder_api::inference::ToolSearchConfig::provider_native();
        request.runtime.tool_search.provider_variant =
            roder_api::inference::ToolSearchProviderVariant::Bm25;

        let body = AnthropicEngine::map_request(&request);

        assert_eq!(body["tools"][0]["type"], "tool_search_tool_bm25_20251119");
        assert_eq!(body["tools"][0]["name"], "tool_search");
        assert!(body["tools"][0].get("defer_loading").is_none());
        assert_eq!(body["tools"][1]["name"], "shell");
        assert_eq!(body["tools"][1]["defer_loading"], true);
        assert_eq!(body["tool_choice"]["type"], "auto");
    }

    #[test]
    fn keeps_explicit_anthropic_tools_when_tool_search_is_default() {
        let body = AnthropicEngine::map_request(&request());

        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["name"], "shell");
        assert!(body["tools"][0].get("defer_loading").is_none());
    }

    #[test]
    fn profile_request_snapshot_maps_anthropic_edit_overlay_and_reasoning() {
        let mut request = request();
        request.instructions.developer = Some(
            "developer\n\n## Model Harness Profile\n\nUse the provided context as the current working set."
                .to_string(),
        );
        request.tools = vec![ToolSpec {
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
        }];
        request.reasoning.level = Some("low".to_string());
        request.runtime.parallel_tool_calls = Some(false);
        request.metadata = json!({
            "modelProfile": {
                "editTool": "edit",
                "schemaPolicy": "standard_required_first",
                "instructionOverlay": "intuitive_context",
                "parallelToolCalls": false
            }
        });

        let body = AnthropicEngine::map_request(&request);

        assert_eq!(
            body["system"][1]["text"],
            request.instructions.developer.unwrap()
        );
        assert_eq!(body["tools"][0]["name"], "edit");
        assert_eq!(
            body["tools"][0]["input_schema"]["required"],
            json!(["path", "old_string", "new_string"])
        );
        assert_eq!(body["output_config"]["effort"], "low");
        assert!(body.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn normalizes_tool_schema_order_for_anthropic_tools() {
        let mut request = request();
        request.tools = vec![ToolSpec {
            name: "shell".to_string(),
            description: "Run shell command".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "additionalProperties": false,
                "required": ["command"]
            }),
        }];

        let body = AnthropicEngine::map_request(&request);
        let schema = serde_json::to_string(&body["tools"][0]["input_schema"]).unwrap();

        assert!(
            schema.starts_with(r#"{"type":"object","required":["command"],"properties":"#),
            "{schema}"
        );
    }

    #[test]
    fn anthropic_capabilities_advertise_streaming_and_prompt_cache() {
        let engine = AnthropicEngine::new("test-key".to_string());
        assert!(engine.capabilities().streaming);
        assert!(engine.capabilities().prompt_cache);
    }

    #[test]
    fn extracts_anthropic_usage_with_cache_components() {
        let value = json!({
            "usage": {
                "input_tokens": 2,
                "cache_creation_input_tokens": 1,
                "cache_read_input_tokens": 8,
                "output_tokens": 7
            }
        });
        assert_eq!(
            extract_usage(&value),
            Some(
                TokenUsage::new(11, 7, 18)
                    .with_cached_prompt_tokens(8)
                    .with_cache_creation_prompt_tokens(1)
            )
        );
    }

    #[test]
    fn sanitizes_dotted_tool_names_for_anthropic() {
        let tools = vec![ToolSpec {
            name: "webwright.run_script".to_string(),
            description: "Run a webwright script".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "script": { "type": "string" } },
                "required": ["script"]
            }),
        }];
        let mut request = request();
        request.tools = tools.clone();
        request.tool_choice = ToolChoice::Specific("webwright.run_script".to_string());

        let body = AnthropicEngine::map_request(&request);
        assert_eq!(body["tools"][0]["name"], "webwright__run_script");
        assert_eq!(body["tool_choice"]["name"], "webwright__run_script");
    }

    fn shell_tool() -> ToolSpec {
        ToolSpec {
            name: "shell".to_string(),
            description: "Run a shell command".to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    fn happy_path_sse_body() -> String {
        [
            r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6","stop_reason":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":1,"cache_read_input_tokens":8,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"ping"}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo "}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"shell","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"cmd\":"}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"ls\"}"}}"#,
            r#"data: {"type":"content_block_stop","index":1}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":7}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]
        .map(|frame| format!("{frame}\n\n"))
        .join("")
    }

    fn happy_path_expected_events() -> Vec<InferenceEvent> {
        vec![
            InferenceEvent::MessageDelta(MessageDelta {
                text: "Hel".to_string(),
                phase: None,
            }),
            InferenceEvent::MessageDelta(MessageDelta {
                text: "lo ".to_string(),
                phase: None,
            }),
            InferenceEvent::MessageDelta(MessageDelta {
                text: "world".to_string(),
                phase: None,
            }),
            InferenceEvent::ToolCallStarted(ToolCallStarted {
                id: "toolu_1".to_string(),
                name: "shell".to_string(),
            }),
            InferenceEvent::ToolCallDelta(ToolCallDelta {
                id: "toolu_1".to_string(),
                arguments_delta: r#"{"cmd":"#.to_string(),
            }),
            InferenceEvent::ToolCallDelta(ToolCallDelta {
                id: "toolu_1".to_string(),
                arguments_delta: r#""ls"}"#.to_string(),
            }),
            InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "toolu_1".to_string(),
                name: "shell".to_string(),
                arguments: r#"{"cmd":"ls"}"#.to_string(),
            }),
            InferenceEvent::Usage(
                TokenUsage::new(11, 7, 18)
                    .with_cached_prompt_tokens(8)
                    .with_cache_creation_prompt_tokens(1),
            ),
            InferenceEvent::ProviderMetadata(json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet-4-6",
                "stop_reason": "tool_use",
                "stop_sequence": null,
                "content": [
                    { "type": "text", "text": "Hello world" },
                    { "type": "tool_use", "id": "toolu_1", "name": "shell", "input": { "cmd": "ls" } }
                ],
                "usage": {
                    "input_tokens": 2,
                    "cache_creation_input_tokens": 1,
                    "cache_read_input_tokens": 8,
                    "output_tokens": 7
                }
            })),
            InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("tool_use".to_string()),
                provider_response_id: Some("msg_1".to_string()),
            }),
        ]
    }

    #[tokio::test]
    async fn streams_multi_frame_happy_path_incrementally() {
        let body = happy_path_sse_body();
        // Hold back everything after the first text delta until the test has
        // observed that delta, proving events flow before the response ends.
        let gate_at = body.find(r#""Hel"}}"#).unwrap() + r#""Hel"}}"#.len() + 2;
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
        let url = spawn_gated_sse_server(
            body.as_bytes()[..gate_at].to_vec(),
            body.as_bytes()[gate_at..].to_vec(),
            gate_rx,
        )
        .await;

        let (response, retry_events) = send_anthropic_request(&url, "secret", &json!({}), None)
            .await
            .unwrap();
        let mut stream = stream_anthropic_sse(response, vec![shell_tool()], retry_events);

        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(
            first,
            InferenceEvent::MessageDelta(MessageDelta {
                text: "Hel".to_string(),
                phase: None,
            })
        );
        gate_tx.send(()).unwrap();

        let mut events = vec![first];
        while let Some(event) = stream.next().await {
            events.push(event.unwrap());
        }
        assert_eq!(events, happy_path_expected_events());
        let text_deltas = events
            .iter()
            .filter(|event| matches!(event, InferenceEvent::MessageDelta(_)))
            .count();
        assert!(
            text_deltas >= 3,
            "expected >=3 text deltas, got {text_deltas}"
        );
    }

    #[tokio::test]
    async fn reassembles_frames_and_multibyte_utf8_split_across_tcp_writes() {
        let body = [
            r#"data: {"type":"message_start","message":{"id":"msg_2","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"héllo "}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"🦀 wörld"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":4}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]
        .map(|frame| format!("{frame}\r\n\r\n"))
        .join("");
        // Split mid-frame AND mid-crab (4-byte scalar) across TCP writes.
        let split = body.find("🦀").unwrap() + 2;
        let url = spawn_sse_server(vec![
            body.as_bytes()[..split].to_vec(),
            body.as_bytes()[split..].to_vec(),
        ])
        .await;

        let (response, retry_events) = send_anthropic_request(&url, "secret", &json!({}), None)
            .await
            .unwrap();
        let events = stream_anthropic_sse(response, Vec::new(), retry_events)
            .collect::<Vec<_>>()
            .await;

        let text = events
            .iter()
            .filter_map(|event| match event {
                Ok(InferenceEvent::MessageDelta(delta)) => Some(delta.text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(text, "héllo 🦀 wörld");
        assert!(matches!(
            events.last(),
            Some(Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some(reason),
                ..
            }))) if reason == "end_turn"
        ));
    }

    #[tokio::test]
    async fn mid_stream_error_frame_ends_stream_with_error() {
        let body = [
            r#"data: {"type":"message_start","message":{"id":"msg_3","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}}"#,
            r#"event: error
data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ]
        .map(|frame| format!("{frame}\n\n"))
        .join("");
        let url = spawn_sse_server(vec![body.into_bytes()]).await;

        let (response, retry_events) = send_anthropic_request(&url, "secret", &json!({}), None)
            .await
            .unwrap();
        let events = stream_anthropic_sse(response, Vec::new(), retry_events)
            .collect::<Vec<_>>()
            .await;

        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].as_ref().unwrap(),
            &InferenceEvent::MessageDelta(MessageDelta {
                text: "partial".to_string(),
                phase: None,
            })
        );
        let error = events[1].as_ref().unwrap_err().to_string();
        assert_eq!(
            error,
            "Anthropic stream error (overloaded_error): Overloaded"
        );
    }

    #[tokio::test]
    async fn stream_closing_before_message_stop_ends_with_error() {
        let body = [
            r#"data: {"type":"message_start","message":{"id":"msg_4","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"cut"}}"#,
        ]
        .map(|frame| format!("{frame}\n\n"))
        .join("");
        let url = spawn_sse_server(vec![body.into_bytes()]).await;

        let (response, retry_events) = send_anthropic_request(&url, "secret", &json!({}), None)
            .await
            .unwrap();
        let events = stream_anthropic_sse(response, Vec::new(), retry_events)
            .collect::<Vec<_>>()
            .await;

        let error = events.last().unwrap().as_ref().unwrap_err().to_string();
        assert_eq!(error, "Anthropic stream closed before message_stop");
    }

    #[tokio::test]
    async fn retry_recovers_after_retryable_status_then_streams() {
        let url = spawn_sse_retry_server(429, r#"{"error":"busy"}"#, happy_path_sse_body()).await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_max_attempts: 2,
            provider_retry_initial_backoff_ms: 0,
            provider_retry_status_codes: vec![429],
            ..ReliabilityRequestPolicy::default()
        };

        let (response, retry_events) =
            send_anthropic_request(&url, "secret", &json!({}), Some(&policy))
                .await
                .unwrap();
        assert_eq!(retry_events.len(), 1);
        assert_eq!(retry_events[0]["kind"], "reliability_retry_attempt");
        assert_eq!(retry_events[0]["errorClass"], "provider_error");
        assert_eq!(retry_events[0]["decision"], "retry");
        assert_eq!(retry_events[0]["cause"], "status_429");

        let events = stream_anthropic_sse(response, vec![shell_tool()], retry_events.clone())
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();

        let mut expected = vec![InferenceEvent::ProviderMetadata(retry_events[0].clone())];
        expected.extend(happy_path_expected_events());
        assert_eq!(events, expected);
    }

    #[tokio::test]
    async fn retry_non_retryable_status_fails_once() {
        let (url, request_count) = spawn_counting_retry_server(vec![
            (400, r#"{"error":"bad request"}"#),
            (
                200,
                r#"{"id":"msg_1","content":[{"type":"text","text":"should-not-run"}]}"#,
            ),
        ])
        .await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_max_attempts: 3,
            provider_retry_initial_backoff_ms: 0,
            provider_retry_status_codes: vec![429],
            ..ReliabilityRequestPolicy::default()
        };

        let err = send_anthropic_request(&url, "secret", &json!({}), Some(&policy))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("Anthropic error 400"));
        assert_eq!(request_count.load(Ordering::SeqCst), 1);
    }

    const SSE_RESPONSE_HEAD: &[u8] =
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";

    /// Serves one SSE response, writing the body in the given raw byte chunks
    /// (which may split frames and multi-byte characters) with a flush between
    /// each.
    async fn spawn_sse_server(chunks: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0_u8; 16384];
            let _ = stream.read(&mut buf).await.unwrap();
            stream.write_all(SSE_RESPONSE_HEAD).await.unwrap();
            for chunk in chunks {
                stream.write_all(&chunk).await.unwrap();
                stream.flush().await.unwrap();
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        format!("http://{addr}/v1/messages")
    }

    /// Serves one SSE response in two parts, holding the second part until the
    /// gate fires.
    async fn spawn_gated_sse_server(
        first: Vec<u8>,
        rest: Vec<u8>,
        gate: tokio::sync::oneshot::Receiver<()>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0_u8; 16384];
            let _ = stream.read(&mut buf).await.unwrap();
            stream.write_all(SSE_RESPONSE_HEAD).await.unwrap();
            stream.write_all(&first).await.unwrap();
            stream.flush().await.unwrap();
            gate.await.unwrap();
            stream.write_all(&rest).await.unwrap();
        });
        format!("http://{addr}/v1/messages")
    }

    /// Responds to the first request with an HTTP error status and to the
    /// second with an SSE stream.
    async fn spawn_sse_retry_server(
        status: u16,
        error_body: &'static str,
        sse_body: String,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0_u8; 16384];
            let _ = stream.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 {status} Error\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{error_body}",
                error_body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            drop(stream);

            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = stream.read(&mut buf).await.unwrap();
            stream.write_all(SSE_RESPONSE_HEAD).await.unwrap();
            stream.write_all(sse_body.as_bytes()).await.unwrap();
        });
        format!("http://{addr}/v1/messages")
    }

    async fn spawn_counting_retry_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let request_count = Arc::new(AtomicUsize::new(0));
        let count = request_count.clone();
        tokio::spawn(async move {
            for (status, body) in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                count.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf).await.unwrap();
                let reason = if status == 200 { "OK" } else { "Error" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{addr}/v1/messages"), request_count)
    }
}
