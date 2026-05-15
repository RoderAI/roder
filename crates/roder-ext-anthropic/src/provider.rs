use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor,
};
use serde_json::json;

pub struct AnthropicEngine {
    api_key: String,
}

impl AnthropicEngine {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait::async_trait]
impl InferenceEngine for AnthropicEngine {
    fn id(&self) -> InferenceEngineId {
        "anthropic".to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: false,
            tool_calls: false,
            parallel_tool_calls: false,
            reasoning_summaries: false,
            structured_output: false,
            image_input: true,
            prompt_cache: true,
            provider_metadata: true,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(vec![ModelDescriptor {
            id: "claude-3-5-sonnet-20241022".to_string(),
            name: "Claude 3.5 Sonnet".to_string(),
            context_window: None,
        }])
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let mut messages = Vec::new();
        for item in &request.conversation {
            match item {
                roder_api::conversation::ConversationItem::UserMessage(m) => {
                    messages.push(json!({ "role": "user", "content": m.text }))
                }
                roder_api::conversation::ConversationItem::AssistantMessage(m) => {
                    messages.push(json!({ "role": "assistant", "content": m.text }))
                }
                roder_api::conversation::ConversationItem::ToolResult(m) => messages.push(
                    json!({ "role": "user", "content": format!("Tool result: {}", m.result) }),
                ),
                _ => {}
            }
        }
        let mut body = json!({
            "model": request.model.model,
            "max_tokens": request.output.max_tokens.unwrap_or(4096),
            "messages": messages,
        });
        if let Some(system) = request.instructions.system {
            body["system"] = json!(system);
        }
        let response = reqwest::Client::new()
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "Anthropic error {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            );
        }
        let value: serde_json::Value = response.json().await?;
        let text = value
            .get("content")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        let stream = futures::stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta { text })),
            Ok(InferenceEvent::ProviderMetadata(value.clone())),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: value
                    .get("stop_reason")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                provider_response_id: value.get("id").and_then(|v| v.as_str()).map(str::to_string),
            })),
        ]);
        Ok(Box::pin(stream))
    }
}
