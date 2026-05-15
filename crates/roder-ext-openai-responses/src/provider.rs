use roder_api::catalog::{PROVIDER_OPENAI, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor, TokenUsage, ToolCallCompleted,
};
use serde_json::{Value, json};

pub struct OpenAiResponsesEngine {
    api_key: String,
    provider_id: String,
    base_url: String,
    headers: Vec<(String, String)>,
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
        Self {
            api_key,
            provider_id: provider_id.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            headers,
        }
    }

    fn map_request(request: &AgentInferenceRequest) -> Value {
        let mut body = json!({
            "model": request.model.model,
            "input": response_input_items(request),
        });
        if let Some(system) = request.instructions.system.as_deref() {
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
            body["reasoning"] = match request.reasoning.level.as_deref() {
                Some(level) => json!({ "effort": level }),
                None => json!({}),
            };
        }
        if !request.tools.is_empty() {
            body["tools"] = json!(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        })
                    })
                    .collect::<Vec<_>>()
            );
            body["tool_choice"] = match &request.tool_choice {
                roder_api::tools::ToolChoice::None => json!("none"),
                roder_api::tools::ToolChoice::Specific(name) => {
                    json!({ "type": "function", "name": name })
                }
                roder_api::tools::ToolChoice::Auto | roder_api::tools::ToolChoice::Any => {
                    json!("auto")
                }
            };
        }
        if let Some(prompt_cache_key) = request.runtime.prompt_cache_key.as_deref() {
            body["prompt_cache_key"] = json!(prompt_cache_key);
        }
        body
    }
}

#[async_trait::async_trait]
impl InferenceEngine for OpenAiResponsesEngine {
    fn id(&self) -> InferenceEngineId {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: false,
            tool_calls: true,
            parallel_tool_calls: false,
            reasoning_summaries: true,
            structured_output: true,
            image_input: false,
            prompt_cache: true,
            provider_metadata: true,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_OPENAI, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let body = Self::map_request(&request);
        let client = reqwest::Client::new();
        let mut request = client
            .post(format!("{}/responses", self.base_url))
            .bearer_auth(&self.api_key);
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }
        let response = request.json(&body).send().await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "OpenAI Responses error {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            );
        }
        let value: Value = response.json().await?;
        let text = extract_response_text(&value);
        let mut events = Vec::new();
        if !text.is_empty() {
            events.push(Ok(InferenceEvent::MessageDelta(MessageDelta { text })));
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
                .get("status")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            provider_response_id: value.get("id").and_then(|v| v.as_str()).map(str::to_string),
        })));
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

fn response_input_items(request: &AgentInferenceRequest) -> Vec<Value> {
    request
        .conversation
        .iter()
        .filter_map(|item| match item {
            roder_api::conversation::ConversationItem::UserMessage(message) => Some(json!({
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": message.text }]
            })),
            roder_api::conversation::ConversationItem::AssistantMessage(message) => Some(json!({
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": message.text }]
            })),
            roder_api::conversation::ConversationItem::ReasoningSummary(summary) => Some(json!({
                "type": "reasoning",
                "summary": [{ "type": "summary_text", "text": summary.text }]
            })),
            roder_api::conversation::ConversationItem::ToolCall(call) => Some(json!({
                "type": "function_call",
                "call_id": call.id,
                "name": call.name,
                "arguments": call.arguments
            })),
            roder_api::conversation::ConversationItem::ToolResult(result) => Some(json!({
                "type": "function_call_output",
                "call_id": result.id,
                "output": result.result
            })),
            roder_api::conversation::ConversationItem::ContextCompaction(compaction) => Some(json!({
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": format!("Context summary:\n{}", compaction.summary) }]
            })),
            _ => None,
        })
        .collect()
}

fn extract_response_text(value: &Value) -> String {
    value
        .get("output_text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| extract_output_text(value))
        .unwrap_or_default()
}

fn extract_tool_calls(value: &Value) -> Vec<ToolCallCompleted> {
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
            let name = item.get("name").and_then(|v| v.as_str())?.to_string();
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
        if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
            for block in content {
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
    (!parts.is_empty()).then(|| parts.join(""))
}

fn extract_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let prompt_tokens = number_to_u32(usage.get("input_tokens"));
    let completion_tokens = number_to_u32(usage.get("output_tokens"));
    let total_tokens = number_to_u32(usage.get("total_tokens"))
        .or_else(|| Some(prompt_tokens? + completion_tokens?));
    Some(TokenUsage {
        prompt_tokens: prompt_tokens.unwrap_or_default(),
        completion_tokens: completion_tokens.unwrap_or_default(),
        total_tokens: total_tokens.unwrap_or_default(),
    })
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::conversation::{AssistantMessage, ConversationItem, UserMessage};
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };

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
            conversation: vec![
                ConversationItem::UserMessage(UserMessage {
                    text: "Hello".to_string(),
                }),
                ConversationItem::AssistantMessage(AssistantMessage {
                    text: "Hi".to_string(),
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
            },
            metadata: json!({}),
        }
    }

    #[test]
    fn maps_responses_request_options_and_input_items() {
        let body = OpenAiResponsesEngine::map_request(&request());
        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["instructions"], "be helpful");
        assert_eq!(body["max_output_tokens"], 200);
        assert!((body["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["prompt_cache_key"], "cache-key");
        assert_eq!(body["text"]["format"]["type"], "json_object");
        assert_eq!(body["tools"][0]["name"], "echo");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][1]["role"], "assistant");
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
        let value = json!({ "usage": { "input_tokens": 3, "output_tokens": 4 } });
        assert_eq!(
            extract_usage(&value),
            Some(TokenUsage {
                prompt_tokens: 3,
                completion_tokens: 4,
                total_tokens: 7,
            })
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
        let calls = extract_tool_calls(&value);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "echo");
        assert_eq!(calls[0].arguments, "{\"text\":\"hello\"}");
    }
}
