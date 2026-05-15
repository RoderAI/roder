use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor,
};
use serde_json::json;

pub struct GeminiEngine {
    api_key: String,
}

impl GeminiEngine {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
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
            tool_calls: false,
            parallel_tool_calls: false,
            reasoning_summaries: false,
            structured_output: true,
            image_input: true,
            prompt_cache: false,
            provider_metadata: true,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(vec![ModelDescriptor {
            id: "gemini-1.5-pro-latest".to_string(),
            name: "Gemini 1.5 Pro".to_string(),
            context_window: None,
        }])
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let text = request
            .conversation
            .iter()
            .filter_map(|item| match item {
                roder_api::conversation::ConversationItem::UserMessage(m) => Some(m.text.as_str()),
                roder_api::conversation::ConversationItem::AssistantMessage(m) => {
                    Some(m.text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let body = json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": text }]
            }]
        });
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
        let value: serde_json::Value = response.json().await?;
        let text = value
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
            .unwrap_or_default();
        let stream = futures::stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta { text })),
            Ok(InferenceEvent::ProviderMetadata(value)),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ]);
        Ok(Box::pin(stream))
    }
}
