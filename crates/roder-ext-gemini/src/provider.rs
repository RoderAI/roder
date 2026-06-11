use roder_api::catalog::{PROVIDER_GEMINI, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceProviderMetadata,
    InferenceTurnContext, MessageDelta, ModelDescriptor, ProviderAuthType, ReasoningDelta,
    TokenUsage, ToolCallCompleted,
};
use roder_api::reliability::{
    ReliabilityRequestPolicy, provider_retry_delay_ms, provider_retry_metadata,
    provider_retry_status_cause,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::media::gemini_user_message_parts;
use crate::schema::gemini_schema;

pub struct GeminiEngine {
    api_key: String,
}

impl GeminiEngine {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    fn map_request(request: &AgentInferenceRequest) -> anyhow::Result<Value> {
        let mut body = json!({
            "contents": gemini_contents(request)?,
        });
        let mut system_parts = Vec::new();
        if let Some(text) = request.instructions.system.as_deref() {
            system_parts.push(json!({ "text": text }));
        }
        if let Some(text) = request.instructions.developer.as_deref() {
            system_parts.push(json!({ "text": text }));
        }
        if !system_parts.is_empty() {
            body["systemInstruction"] = json!({ "parts": system_parts });
        }
        let generation_config = gemini_generation_config(request);
        if !generation_config
            .as_object()
            .is_some_and(|object| object.is_empty())
        {
            body["generationConfig"] = generation_config;
        }
        if !request.tools.is_empty() {
            body["tools"] = json!([{
                "functionDeclarations": request
                    .tools
                    .iter()
                    .map(|tool| {
                        let tool = tool.normalized_for_model(roder_api::ToolSchemaPolicy::warning());
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": gemini_schema(tool.parameters),
                        })
                    })
                    .collect::<Vec<_>>()
            }]);
            body["toolConfig"] = gemini_tool_config(&request.tool_choice);
        }
        Ok(body)
    }
}

#[async_trait::async_trait]
impl InferenceEngine for GeminiEngine {
    fn id(&self) -> InferenceEngineId {
        "gemini".to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: false,
            tool_calls: true,
            parallel_tool_calls: false,
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
            name: "Google".to_string(),
            description: Some("Gemini API key provider".to_string()),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some("API key".to_string()),
            auth_configured: Some(true),
            recommended: false,
            sort_order: 40,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_GEMINI, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let body = Self::map_request(&request)?;
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            request.model.model, self.api_key
        );
        let (value, retry_events) =
            send_gemini_request(&url, &body, request.runtime.reliability.as_ref()).await?;
        let text = extract_candidate_text(&value);
        let reasoning = extract_candidate_thinking(&value);
        let mut events = Vec::new();
        if !reasoning.is_empty() {
            events.push(Ok(InferenceEvent::ReasoningDelta(ReasoningDelta {
                text: reasoning,
            })));
        }
        if !text.is_empty() {
            events.push(Ok(InferenceEvent::MessageDelta(MessageDelta {
                text,
                phase: None,
            })));
        }
        for call in extract_tool_calls(&value) {
            events.push(Ok(InferenceEvent::ToolCallCompleted(call)));
        }
        if let Some(usage) = extract_usage(&value) {
            events.push(Ok(InferenceEvent::Usage(usage)));
        }
        for retry_event in retry_events {
            events.push(Ok(InferenceEvent::ProviderMetadata(retry_event)));
        }
        events.push(Ok(InferenceEvent::ProviderMetadata(value.clone())));
        events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: value
                .pointer("/candidates/0/finishReason")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            provider_response_id: value
                .get("responseId")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })));
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

async fn send_gemini_request(
    url: &str,
    body: &Value,
    policy: Option<&ReliabilityRequestPolicy>,
) -> anyhow::Result<(Value, Vec<Value>)> {
    let policy = policy.cloned().unwrap_or_default();
    let attempts = policy.provider_retry_max_attempts.max(1);
    let client = reqwest::Client::new();
    let mut last_error = None;
    let mut retry_events = Vec::new();
    for attempt in 1..=attempts {
        let response = client.post(url).json(body).send().await;
        match response {
            Ok(response) if response.status().is_success() => {
                let bytes = response.bytes().await?;
                if bytes.is_empty() && policy.retry_empty_provider_body && attempt < attempts {
                    push_retry_event(&mut retry_events, attempt, "empty_provider_body", &policy);
                    retry_sleep(&policy, attempt).await;
                    continue;
                }
                return Ok((serde_json::from_slice(&bytes)?, retry_events));
            }
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                let retryable = policy
                    .provider_retry_status_codes
                    .contains(&status.as_u16());
                last_error = Some(format!("Gemini error {status}: {text}"));
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
    anyhow::bail!(last_error.unwrap_or_else(|| "Gemini request failed".to_string()))
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

fn gemini_contents(request: &AgentInferenceRequest) -> anyhow::Result<Vec<Value>> {
    let mut contents = Vec::new();
    let mut replay = GeminiProviderReplay::default();
    for item in &request.transcript {
        match item {
            roder_api::transcript::TranscriptItem::UserMessage(message) => {
                contents.push(json!({
                    "role": "user",
                    "parts": gemini_user_message_parts(message)?
                }));
            }
            roder_api::transcript::TranscriptItem::AssistantMessage(message) => {
                contents.push(json!({
                    "role": "model",
                    "parts": [{ "text": message.text }]
                }));
            }
            roder_api::transcript::TranscriptItem::ToolCall(call) => {
                if replay.should_skip_tool_call(call) {
                    continue;
                }
                contents.push(json!({
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "id": call.id,
                            "name": call.name,
                            "args": parse_json_object(&call.arguments)
                        }
                    }]
                }));
            }
            roder_api::transcript::TranscriptItem::ToolResult(result) => {
                contents.push(json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "id": result.id,
                            "name": result.name.clone().unwrap_or_default(),
                            "response": { "result": result.result, "is_error": result.is_error }
                        }
                    }]
                }));
            }
            roder_api::transcript::TranscriptItem::ProviderMetadata(metadata) => {
                append_provider_function_call_content(metadata, &mut contents, &mut replay);
            }
            _ => {}
        }
    }
    Ok(contents)
}

#[derive(Default)]
struct GeminiProviderReplay {
    call_ids: HashSet<String>,
    call_keys: HashMap<(String, String), usize>,
}

impl GeminiProviderReplay {
    fn record_part(&mut self, part: &Value) {
        let Some(call) = part.get("functionCall") else {
            return;
        };
        if let Some(id) = call.get("id").and_then(Value::as_str)
            && !id.is_empty()
        {
            self.call_ids.insert(id.to_string());
        }
        if let Some(name) = call.get("name").and_then(Value::as_str) {
            let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
            *self
                .call_keys
                .entry((name.to_string(), canonical_json(&args)))
                .or_default() += 1;
        }
    }

    fn should_skip_tool_call(&mut self, call: &roder_api::transcript::ToolCallRecord) -> bool {
        if !call.id.is_empty() && self.call_ids.remove(&call.id) {
            return true;
        }
        let key = (call.name.clone(), canonical_tool_arguments(&call.arguments));
        let Some(count) = self.call_keys.get_mut(&key) else {
            return false;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.call_keys.remove(&key);
        }
        true
    }
}

fn append_provider_function_call_content(
    metadata: &Value,
    contents: &mut Vec<Value>,
    replay: &mut GeminiProviderReplay,
) {
    let Some(content) = metadata.pointer("/candidates/0/content") else {
        return;
    };
    let Some(parts) = content.get("parts").and_then(Value::as_array) else {
        return;
    };
    if !parts.iter().any(|part| part.get("functionCall").is_some()) {
        return;
    }
    for part in parts {
        replay.record_part(part);
    }
    let mut content = content.clone();
    if content.get("role").is_none() {
        content["role"] = json!("model");
    }
    contents.push(content);
}

fn canonical_tool_arguments(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .ok()
        .unwrap_or_else(|| json!({}))
        .to_string()
}

fn canonical_json(value: &Value) -> String {
    value.to_string()
}

fn gemini_tool_config(choice: &roder_api::tools::ToolChoice) -> Value {
    let mode = match choice {
        roder_api::tools::ToolChoice::Auto => "AUTO",
        roder_api::tools::ToolChoice::Any => "ANY",
        roder_api::tools::ToolChoice::None => "NONE",
        roder_api::tools::ToolChoice::Specific(_) => "ANY",
    };
    let mut function_calling_config = json!({ "mode": mode });
    if let roder_api::tools::ToolChoice::Specific(name) = choice {
        function_calling_config["allowedFunctionNames"] = json!([name]);
    }
    json!({ "functionCallingConfig": function_calling_config })
}

fn gemini_generation_config(request: &AgentInferenceRequest) -> Value {
    let mut config = json!({});
    if let Some(max_tokens) = request.output.max_tokens {
        config["maxOutputTokens"] = json!(max_tokens);
    }
    if let Some(temperature) = request.output.temperature {
        config["temperature"] = json!(temperature);
    }
    if let Some(top_p) = request.output.top_p {
        config["topP"] = json!(top_p);
    }
    if let Some(response_format) = request.output.response_format.as_ref() {
        if let Some(mime_type) = response_format.get("mime_type").and_then(|v| v.as_str()) {
            config["responseMimeType"] = json!(mime_type);
        } else if response_format.get("type").and_then(|v| v.as_str()) == Some("json_object") {
            config["responseMimeType"] = json!("application/json");
        }
        if let Some(schema) = response_format.get("schema") {
            config["responseSchema"] = schema.clone();
        }
    }
    if request.reasoning.enabled
        && let Some(thinking_config) = gemini_thinking_config(request.reasoning.level.as_deref())
    {
        config["thinkingConfig"] = thinking_config;
    }
    config
}

fn gemini_thinking_config(level: Option<&str>) -> Option<Value> {
    match level {
        Some("minimal") => Some(json!({ "thinkingLevel": "MINIMAL" })),
        Some("low") => Some(json!({ "thinkingLevel": "LOW" })),
        Some("medium") => Some(json!({ "thinkingLevel": "MEDIUM" })),
        Some("high") => Some(json!({ "thinkingLevel": "HIGH" })),
        None => None,
        Some(_) => None,
    }
}

fn parse_json_object(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(|value| value.is_object())
        .unwrap_or_else(|| json!({}))
}

fn extract_candidate_text(value: &Value) -> String {
    value
        .get("candidates")
        .and_then(|v| v.as_array())
        .and_then(|items| items.first())
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(|parts| parts.as_array())
        .map(|parts| {
            parts
                .iter()
                // Skip parts that are marked as thought/reasoning
                .filter(|part| {
                    let is_thought = part
                        .get("thought")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    !is_thought && part.get("thought").and_then(|v| v.as_str()).is_none()
                })
                .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn extract_candidate_thinking(value: &Value) -> String {
    value
        .get("candidates")
        .and_then(|v| v.as_array())
        .and_then(|items| items.first())
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(|parts| parts.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| {
                    if let Some(thought_val) = part.get("thought") {
                        if thought_val.as_bool().unwrap_or(false) {
                            return part.get("text").and_then(|v| v.as_str());
                        }
                        if let Some(thought_str) = thought_val.as_str() {
                            return Some(thought_str);
                        }
                    }
                    None
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn extract_tool_calls(value: &Value) -> Vec<ToolCallCompleted> {
    value
        .pointer("/candidates/0/content/parts")
        .and_then(|v| v.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("functionCall"))
                .filter_map(|call| {
                    let id = call
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = call.get("name").and_then(|v| v.as_str())?.to_string();
                    let arguments = call
                        .get("args")
                        .map(|args| args.to_string())
                        .unwrap_or_else(|| "{}".to_string());
                    Some(ToolCallCompleted {
                        id,
                        name,
                        arguments,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usageMetadata")?;
    Some(
        TokenUsage::new(
            number_to_u32(usage.get("promptTokenCount")).unwrap_or_default(),
            number_to_u32(usage.get("candidatesTokenCount")).unwrap_or_default(),
            number_to_u32(usage.get("totalTokenCount")).unwrap_or_default(),
        )
        .with_cached_prompt_tokens(
            number_to_u32(usage.get("cachedContentTokenCount")).unwrap_or_default(),
        ),
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
    use roder_api::reliability::ReliabilityRequestPolicy;
    use roder_api::tools::{ToolChoice, ToolSpec};
    use roder_api::transcript::{
        AssistantMessage, InputImage, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: "gemini".to_string(),
                model: "gemini-3.1-pro-preview".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("system".to_string()),
                developer: Some("developer".to_string()),
                developer_context: None,
            },
            transcript: vec![
                TranscriptItem::UserMessage(UserMessage::text("Hello")),
                TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "Hi".to_string(),
                    phase: None,
                }),
                TranscriptItem::ToolCall(ToolCallRecord {
                    id: "call_1".to_string(),
                    name: "shell".to_string(),
                    arguments: r#"{"cmd":"pwd"}"#.to_string(),
                }),
                TranscriptItem::ToolResult(ToolResultRecord {
                    id: "call_1".to_string(),
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
            tool_choice: ToolChoice::Specific("shell".to_string()),
            reasoning: ReasoningConfig {
                enabled: true,
                level: Some("medium".to_string()),
            },
            output: OutputConfig {
                max_tokens: Some(128),
                temperature: Some(0.4),
                top_p: Some(0.95),
                response_format: Some(json!({
                    "type": "json_object",
                    "schema": { "type": "object" }
                })),
            },
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        }
    }

    #[test]
    fn maps_gemini_request() {
        let body = GeminiEngine::map_request(&request()).unwrap();
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "system");
        assert_eq!(body["systemInstruction"]["parts"][1]["text"], "developer");
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(
            body["contents"][2]["parts"][0]["functionCall"]["name"],
            "shell"
        );
        assert_eq!(
            body["contents"][3]["parts"][0]["functionResponse"]["response"]["result"],
            "ok"
        );
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 128);
        assert!((body["generationConfig"]["temperature"].as_f64().unwrap() - 0.4).abs() < 1e-6);
        assert!((body["generationConfig"]["topP"].as_f64().unwrap() - 0.95).abs() < 1e-6);
        assert_eq!(
            body["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert_eq!(body["generationConfig"]["responseSchema"]["type"], "object");
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "MEDIUM"
        );
        assert_eq!(body["tools"][0]["functionDeclarations"][0]["name"], "shell");
        assert_eq!(
            body["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"][0],
            "shell"
        );
    }

    #[test]
    fn maps_user_images_to_gemini_inline_data_parts() {
        let mut request = request();
        request.transcript = vec![TranscriptItem::UserMessage(UserMessage::with_images(
            "what is shown?",
            vec![InputImage {
                image_url: "data:image/png;base64,YWJj".to_string(),
            }],
        ))];

        let body = GeminiEngine::map_request(&request).unwrap();

        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "what is shown?");
        assert_eq!(
            body["contents"][0]["parts"][1]["inline_data"]["mime_type"],
            "image/png"
        );
        assert_eq!(
            body["contents"][0]["parts"][1]["inline_data"]["data"],
            "YWJj"
        );
        assert!(
            GeminiEngine::new("key".to_string())
                .capabilities()
                .image_input
        );
    }

    #[test]
    fn profile_request_snapshot_maps_gemini_edit_overlay_and_thinking() {
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
        request.tool_choice = ToolChoice::Specific("edit".to_string());
        request.reasoning.level = Some("high".to_string());
        request.runtime.parallel_tool_calls = Some(false);
        request.metadata = json!({
            "modelProfile": {
                "editTool": "edit",
                "schemaPolicy": "standard_required_first",
                "instructionOverlay": "intuitive_context",
                "parallelToolCalls": false
            }
        });

        let body = GeminiEngine::map_request(&request).unwrap();

        assert_eq!(
            body["systemInstruction"]["parts"][1]["text"],
            request.instructions.developer.unwrap()
        );
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "HIGH"
        );
        assert_eq!(body["tools"][0]["functionDeclarations"][0]["name"], "edit");
        assert_eq!(
            body["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"][0],
            "edit"
        );
        assert!(body.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn normalizes_tool_schema_order_for_gemini_tools() {
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

        let body = GeminiEngine::map_request(&request).unwrap();
        let schema =
            serde_json::to_string(&body["tools"][0]["functionDeclarations"][0]["parameters"])
                .unwrap();

        assert!(
            schema.starts_with(r#"{"type":"object","required":["command"],"properties":"#),
            "{schema}"
        );
    }

    #[test]
    fn replays_gemini_function_call_content_with_thought_signature() {
        let mut request = request();
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("List files")),
            TranscriptItem::ProviderMetadata(json!({
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{
                            "functionCall": {
                                "name": "shell",
                                "args": { "cmd": "ls" }
                            },
                            "thoughtSignature": "signed-thought"
                        }]
                    }
                }]
            })),
            TranscriptItem::ToolCall(ToolCallRecord {
                id: "call_1".to_string(),
                name: "shell".to_string(),
                arguments: r#"{"cmd":"ls"}"#.to_string(),
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "call_1".to_string(),
                name: Some("shell".to_string()),
                result: "Cargo.toml".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];

        let body = GeminiEngine::map_request(&request).unwrap();
        let contents = body["contents"].as_array().unwrap();
        let function_call_count = contents
            .iter()
            .filter(|content| {
                content
                    .pointer("/parts/0/functionCall/name")
                    .and_then(Value::as_str)
                    == Some("shell")
            })
            .count();

        assert_eq!(function_call_count, 1);
        assert_eq!(
            contents[1]["parts"][0]["thoughtSignature"],
            "signed-thought"
        );
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["name"], "shell");
    }

    #[test]
    fn rejects_unsupported_gemini_thinking_levels() {
        assert_eq!(gemini_thinking_config(Some("xhigh")), None);
        assert_eq!(gemini_thinking_config(Some("none")), None);
        assert_eq!(gemini_thinking_config(None), None);
    }

    #[test]
    fn extracts_gemini_text_tool_calls_usage_and_metadata() {
        let value = json!({
            "responseId": "resp_123",
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [
                    { "text": "hello" },
                    { "functionCall": { "id": "call_2", "name": "shell", "args": { "cmd": "ls" } } },
                    { "text": " world" }
                ] }
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 3,
                "totalTokenCount": 13,
                "cachedContentTokenCount": 9
            }
        });
        assert_eq!(extract_candidate_text(&value), "hello world");
        assert_eq!(
            extract_tool_calls(&value),
            vec![ToolCallCompleted {
                id: "call_2".to_string(),
                name: "shell".to_string(),
                arguments: r#"{"cmd":"ls"}"#.to_string(),
            }]
        );
        assert_eq!(
            extract_usage(&value),
            Some(TokenUsage::new(10, 3, 13).with_cached_prompt_tokens(9))
        );
    }

    #[tokio::test]
    async fn retry_recovers_after_empty_body() {
        let url = spawn_retry_server(vec![
            (200, ""),
            (
                200,
                r#"{"responseId":"resp_1","candidates":[{"finishReason":"STOP","content":{"parts":[{"text":"ok"}]}}]}"#,
            ),
        ])
        .await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_max_attempts: 2,
            provider_retry_initial_backoff_ms: 0,
            retry_empty_provider_body: true,
            ..ReliabilityRequestPolicy::default()
        };

        let (value, retry_events) = send_gemini_request(&url, &json!({}), Some(&policy))
            .await
            .unwrap();

        assert_eq!(extract_candidate_text(&value), "ok");
        assert_eq!(retry_events[0]["kind"], "reliability_retry_attempt");
    }

    #[test]
    fn extracts_gemini_thinking_blocks_separately_from_text() {
        // Test Case A: Part containing a "thought" string directly (some Gemini API versions)
        let val_a = json!({
            "candidates": [{
                "content": { "parts": [
                    { "thought": "I should search for files." },
                    { "text": "I will search now." }
                ] }
            }]
        });
        assert_eq!(extract_candidate_text(&val_a), "I will search now.");
        assert_eq!(
            extract_candidate_thinking(&val_a),
            "I should search for files."
        );

        // Test Case B: Part containing "text" with "thought": true (other Gemini API versions/schemas)
        let val_b = json!({
            "candidates": [{
                "content": { "parts": [
                    { "text": "Analyzing repo structure...", "thought": true },
                    { "text": "Let's use the glob tool." }
                ] }
            }]
        });
        assert_eq!(extract_candidate_text(&val_b), "Let's use the glob tool.");
        assert_eq!(
            extract_candidate_thinking(&val_b),
            "Analyzing repo structure..."
        );
    }

    async fn spawn_retry_server(responses: Vec<(u16, &'static str)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf).await.unwrap();
                let reason = if status == 200 { "OK" } else { "Retry" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        format!("http://{addr}/v1beta/models/test:generateContent?key=secret")
    }
}
