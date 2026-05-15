use futures::{stream, Stream, StreamExt};
use reqwest_eventsource::{Event, EventSource, RequestBuilderExt};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::pin::Pin;

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

pub struct OpenAiResponsesEngine {
    api_key: String,
}

impl OpenAiResponsesEngine {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait::async_trait]
impl InferenceEngine for OpenAiResponsesEngine {
    fn id(&self) -> InferenceEngineId {
        "openai-responses".to_string()
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
                id: "gpt-5.5".to_string(),
                name: "GPT-5.5".to_string(),
            },
        ])
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let mut messages = Vec::new();
        if let Some(sys) = &request.instructions.system {
            messages.push(json!({ "role": "system", "content": sys }));
        }
        for item in &request.conversation {
            match item {
                roder_api::conversation::ConversationItem::UserMessage(m) => {
                    messages.push(json!({ "role": "user", "content": m.text }));
                }
                roder_api::conversation::ConversationItem::AssistantMessage(m) => {
                    messages.push(json!({ "role": "assistant", "content": m.text }));
                }
                _ => {}
            }
        }

        let body = json!({
            "model": request.model.model,
            "messages": messages,
            "stream": true,
        });

        let client = reqwest::Client::new();
        let mut es = client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .eventsource()?;

        let stream = async_stream::stream! {
            while let Some(event) = es.next().await {
                match event {
                    Ok(Event::Open) => continue,
                    Ok(Event::Message(msg)) => {
                        if msg.data == "[DONE]" {
                            yield Ok(InferenceEvent::Completed(CompletionMetadata {
                                stop_reason: Some("stop".to_string()),
                            }));
                            break;
                        }
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&msg.data) {
                            if let Some(choices) = v.get("choices").and_then(|c| c.as_array()) {
                                if let Some(choice) = choices.first() {
                                    if let Some(delta) = choice.get("delta") {
                                        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                            yield Ok(InferenceEvent::MessageDelta(MessageDelta {
                                                text: content.to_string(),
                                            }));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        yield Err(anyhow::anyhow!("Stream error: {}", e));
                        break;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }
}
