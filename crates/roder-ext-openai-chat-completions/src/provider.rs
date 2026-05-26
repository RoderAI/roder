use roder_api::catalog::{PROVIDER_OPENAI, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor, TokenUsage,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
}

pub struct OpenAiChatCompletionsEngine {
    api_key: String,
}

impl OpenAiChatCompletionsEngine {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    pub fn map_request(&self, req: &AgentInferenceRequest) -> ChatRequest {
        let mut messages = Vec::new();
        if let Some(sys) = &req.instructions.system {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: sys.clone(),
            });
        }
        if let Some(dev) = &req.instructions.developer {
            messages.push(ChatMessage {
                role: "developer".to_string(),
                content: dev.clone(),
            });
        }
        for item in &req.transcript {
            match item {
                roder_api::transcript::TranscriptItem::UserMessage(m) => {
                    messages.push(ChatMessage {
                        role: "user".to_string(),
                        content: m.text.clone(),
                    });
                }
                roder_api::transcript::TranscriptItem::AssistantMessage(m) => {
                    messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: m.text.clone(),
                    });
                }
                roder_api::transcript::TranscriptItem::ToolResult(m) => {
                    messages.push(ChatMessage {
                        role: "tool".to_string(),
                        content: m.result.clone(),
                    });
                }
                _ => {}
            }
        }
        ChatRequest {
            model: req.model.model.clone(),
            messages,
            max_tokens: req.output.max_tokens,
            temperature: req.output.temperature,
            top_p: req.output.top_p,
            response_format: req.output.response_format.clone(),
        }
    }
}

#[async_trait::async_trait]
impl InferenceEngine for OpenAiChatCompletionsEngine {
    fn id(&self) -> InferenceEngineId {
        "openai-chat-completions".to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: false,
            tool_calls: false,
            parallel_tool_calls: false,
            reasoning_summaries: false,
            structured_output: true,
            image_input: false,
            prompt_cache: false,
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
        let chat_req = self.map_request(&request);
        let mut body = serde_json::to_value(chat_req)?;
        body["stream"] = json!(false);
        let response = reqwest::Client::new()
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "OpenAI Chat Completions error {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            );
        }
        let value: Value = response.json().await?;
        let text = extract_message_text(&value);
        let mut events = vec![Ok(InferenceEvent::MessageDelta(MessageDelta {
            text,
            phase: None,
        }))];
        if let Some(usage) = extract_usage(&value) {
            events.push(Ok(InferenceEvent::Usage(usage)));
        }
        events.push(Ok(InferenceEvent::ProviderMetadata(value.clone())));
        events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: value
                .pointer("/choices/0/finish_reason")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            provider_response_id: value.get("id").and_then(|v| v.as_str()).map(str::to_string),
        })));
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

fn extract_message_text(value: &Value) -> String {
    value
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn extract_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let cached_prompt_tokens = number_to_u32(usage.pointer("/prompt_tokens_details/cached_tokens"))
        .or_else(|| number_to_u32(usage.pointer("/input_tokens_details/cached_tokens")))
        .unwrap_or_default();
    Some(
        TokenUsage::new(
            number_to_u32(usage.get("prompt_tokens")).unwrap_or_default(),
            number_to_u32(usage.get("completion_tokens")).unwrap_or_default(),
            number_to_u32(usage.get("total_tokens")).unwrap_or_default(),
        )
        .with_cached_prompt_tokens(cached_prompt_tokens),
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
    use roder_api::transcript::{AssistantMessage, TranscriptItem, UserMessage};

    fn request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("You are a helpful assistant.".to_string()),
                developer: Some("Be concise.".to_string()),
            },
            transcript: vec![
                TranscriptItem::UserMessage(UserMessage::text("Hello")),
                TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "Hi there!".to_string(),
                    phase: None,
                }),
            ],
            tools: vec![],
            tool_choice: roder_api::tools::ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig {
                max_tokens: Some(50),
                temperature: Some(0.1),
                top_p: Some(0.9),
                response_format: Some(json!({ "type": "json_object" })),
            },
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        }
    }

    #[test]
    fn test_map_request() {
        let engine = OpenAiChatCompletionsEngine::new("test_key".to_string());
        let chat_req = engine.map_request(&request());
        assert_eq!(chat_req.model, "gpt-4o");
        assert_eq!(chat_req.messages.len(), 4);
        assert_eq!(chat_req.messages[0].role, "system");
        assert_eq!(chat_req.messages[1].role, "developer");
        assert_eq!(chat_req.messages[2].role, "user");
        assert_eq!(chat_req.messages[3].role, "assistant");
        assert_eq!(chat_req.max_tokens, Some(50));
        assert_eq!(
            chat_req.response_format,
            Some(json!({ "type": "json_object" }))
        );
    }

    #[test]
    fn extracts_chat_completion_text_usage_and_finish_reason() {
        let value = json!({
            "id": "chatcmpl_123",
            "choices": [{
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "hello" }
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 3,
                "total_tokens": 13,
                "prompt_tokens_details": { "cached_tokens": 9 }
            }
        });
        assert_eq!(extract_message_text(&value), "hello");
        assert_eq!(
            extract_usage(&value),
            Some(TokenUsage::new(10, 3, 13).with_cached_prompt_tokens(9))
        );
    }
}
