use std::pin::Pin;
use futures::{Stream, stream};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor,
};
use serde::{Deserialize, Serialize};

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
            supports_tools: true,
            supports_vision: true,
            supports_reasoning: false,
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
            },
            ModelDescriptor {
                id: "gpt-4-turbo".to_string(),
                name: "GPT-4 Turbo".to_string(),
            },
        ])
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let _chat_req = self.map_request(&request);
        // Normally we'd do a reqwest POST to /v1/chat/completions here.
        // For the milestone stub, we emit a completed fake response.

        let stream = stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "Stub response from chat completions".to_string(),
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
            })),
        ]);

        Ok(Box::pin(stream))
    }
}
