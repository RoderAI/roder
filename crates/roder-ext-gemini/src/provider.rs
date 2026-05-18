use roder_api::catalog::{PROVIDER_GEMINI, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceProviderMetadata,
    InferenceTurnContext, MessageDelta, ModelDescriptor, ProviderAuthType, TokenUsage,
    ToolCallCompleted,
};
use serde_json::{Value, json};

pub struct GeminiEngine {
    api_key: String,
}

impl GeminiEngine {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    fn map_request(request: &AgentInferenceRequest) -> Value {
        let mut body = json!({
            "contents": gemini_contents(request),
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
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        })
                    })
                    .collect::<Vec<_>>()
            }]);
            body["toolConfig"] = gemini_tool_config(&request.tool_choice);
        }
        body
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
            image_input: false,
            prompt_cache: false,
            provider_metadata: true,
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
        let body = Self::map_request(&request);
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            request.model.model, self.api_key
        );
        let response = reqwest::Client::new().post(url).json(&body).send().await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "Gemini error {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            );
        }
        let value: Value = response.json().await?;
        let text = extract_candidate_text(&value);
        let mut events = Vec::new();
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

fn gemini_contents(request: &AgentInferenceRequest) -> Vec<Value> {
    request
        .conversation
        .iter()
        .filter_map(|item| match item {
            roder_api::conversation::ConversationItem::UserMessage(message) => Some(json!({
                "role": "user",
                "parts": [{ "text": message.text }]
            })),
            roder_api::conversation::ConversationItem::AssistantMessage(message) => Some(json!({
                "role": "model",
                "parts": [{ "text": message.text }]
            })),
            roder_api::conversation::ConversationItem::ToolCall(call) => Some(json!({
                "role": "model",
                "parts": [{
                    "functionCall": {
                        "id": call.id,
                        "name": call.name,
                        "args": parse_json_object(&call.arguments)
                    }
                }]
            })),
            roder_api::conversation::ConversationItem::ToolResult(result) => Some(json!({
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "id": result.id,
                        "name": result.name.clone().unwrap_or_default(),
                        "response": { "result": result.result, "is_error": result.is_error }
                    }
                }]
            })),
            _ => None,
        })
        .collect()
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
        Some("none") => Some(json!({ "thinkingBudget": 0 })),
        Some("minimal") => Some(json!({ "thinkingLevel": "MINIMAL" })),
        Some("low") => Some(json!({ "thinkingLevel": "LOW" })),
        Some("medium") => Some(json!({ "thinkingLevel": "MEDIUM" })),
        Some("high") | Some("xhigh") | None => Some(json!({ "thinkingLevel": "HIGH" })),
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
                .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
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
    Some(TokenUsage {
        prompt_tokens: number_to_u32(usage.get("promptTokenCount")).unwrap_or_default(),
        completion_tokens: number_to_u32(usage.get("candidatesTokenCount")).unwrap_or_default(),
        total_tokens: number_to_u32(usage.get("totalTokenCount")).unwrap_or_default(),
    })
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::conversation::{
        AssistantMessage, ConversationItem, ToolCallRecord, ToolResultRecord, UserMessage,
    };
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };
    use roder_api::tools::{ToolChoice, ToolSpec};

    fn request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: "gemini".to_string(),
                model: "gemini-3.1-pro-preview".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("system".to_string()),
                developer: Some("developer".to_string()),
            },
            conversation: vec![
                ConversationItem::UserMessage(UserMessage::text("Hello")),
                ConversationItem::AssistantMessage(AssistantMessage {
                    text: "Hi".to_string(),
                    phase: None,
                }),
                ConversationItem::ToolCall(ToolCallRecord {
                    id: "call_1".to_string(),
                    name: "shell".to_string(),
                    arguments: r#"{"cmd":"pwd"}"#.to_string(),
                }),
                ConversationItem::ToolResult(ToolResultRecord {
                    id: "call_1".to_string(),
                    name: Some("shell".to_string()),
                    result: "ok".to_string(),
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
        let body = GeminiEngine::map_request(&request());
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
                "promptTokenCount": 2,
                "candidatesTokenCount": 3,
                "totalTokenCount": 5
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
            Some(TokenUsage {
                prompt_tokens: 2,
                completion_tokens: 3,
                total_tokens: 5,
            })
        );
    }
}
