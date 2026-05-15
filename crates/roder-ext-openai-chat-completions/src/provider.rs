use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
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
        for item in &req.conversation {
            match item {
                roder_api::conversation::ConversationItem::UserMessage(m) => {
                    messages.push(ChatMessage {
                        role: "user".to_string(),
                        content: m.text.clone(),
                    });
                }
                roder_api::conversation::ConversationItem::AssistantMessage(m) => {
                    messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: m.text.clone(),
                    });
                }
                roder_api::conversation::ConversationItem::ToolResult(m) => {
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
            structured_output: false,
            image_input: true,
            prompt_cache: false,
            provider_metadata: true,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(vec![
            ModelDescriptor {
                id: "gpt-4o".to_string(),
                name: "GPT-4o".to_string(),
                context_window: None,
            },
            ModelDescriptor {
                id: "gpt-4-turbo".to_string(),
                name: "GPT-4 Turbo".to_string(),
                context_window: None,
            },
        ])
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let chat_req = self.map_request(&request);
        let body = json!({
            "model": chat_req.model,
            "messages": chat_req.messages,
            "stream": false,
        });
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
        let value: serde_json::Value = response.json().await?;
        let text = value
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let usage = value.get("usage").cloned();
        let stream = futures::stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta { text })),
            usage
                .map(InferenceEvent::ProviderMetadata)
                .map(Ok)
                .unwrap_or_else(|| Ok(InferenceEvent::ProviderMetadata(json!({})))),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: value.get("id").and_then(|v| v.as_str()).map(str::to_string),
            })),
        ]);
        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::conversation::{AssistantMessage, ConversationItem, UserMessage};
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };

    #[test]
    fn test_map_request() {
        let engine = OpenAiChatCompletionsEngine::new("test_key".to_string());
        let req = AgentInferenceRequest {
            model: ModelSelection {
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("You are a helpful assistant.".to_string()),
                developer: None,
            },
            conversation: vec![
                ConversationItem::UserMessage(UserMessage {
                    text: "Hello".to_string(),
                }),
                ConversationItem::AssistantMessage(AssistantMessage {
                    text: "Hi there!".to_string(),
                }),
            ],
            tools: vec![],
            tool_choice: roder_api::tools::ToolChoice::Auto,
            reasoning: ReasoningConfig {
                enabled: false,
                level: None,
            },
            output: OutputConfig {
                max_tokens: None,
                temperature: None,
                top_p: None,
                response_format: None,
            },
            runtime: RuntimeHints {
                trace_id: None,
                prompt_cache_key: None,
            },
            metadata: serde_json::json!({}),
        };
        let chat_req = engine.map_request(&req);
        assert_eq!(chat_req.model, "gpt-4o");
        assert_eq!(chat_req.messages.len(), 3);
        assert_eq!(chat_req.messages[0].role, "system");
        assert_eq!(chat_req.messages[1].role, "user");
        assert_eq!(chat_req.messages[2].role, "assistant");
    }
}
