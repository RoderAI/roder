use roder_api::catalog::{PROVIDER_ANTHROPIC, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext, ModelDescriptor,
    ProviderAuthType, TokenUsage, ToolSearchProviderVariant,
};
use serde_json::{Value, json};

use crate::stream::{
    ANTHROPIC_MESSAGES_URL, AnthropicTurnRequest, anthropic_stream_client, start_anthropic_stream,
};

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
        let turn = AnthropicTurnRequest {
            client: anthropic_stream_client()?,
            url: ANTHROPIC_MESSAGES_URL.to_string(),
            api_key: self.api_key.clone(),
            body: Self::map_request(&request),
            policy: request.runtime.reliability.clone().unwrap_or_default(),
        };
        start_anthropic_stream(turn, request.tools).await
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
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };
    use roder_api::tools::{ToolChoice, ToolSpec};
    use roder_api::transcript::{
        AssistantMessage, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
    };

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
}
