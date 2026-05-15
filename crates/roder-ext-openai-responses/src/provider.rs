use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor,
};
use serde_json::json;

pub struct OpenAiResponsesEngine {
    api_key: String,
}

impl OpenAiResponsesEngine {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    fn input_text(req: &AgentInferenceRequest) -> String {
        req.conversation
            .iter()
            .filter_map(|item| match item {
                roder_api::conversation::ConversationItem::UserMessage(m) => {
                    Some(format!("user: {}", m.text))
                }
                roder_api::conversation::ConversationItem::AssistantMessage(m) => {
                    Some(format!("assistant: {}", m.text))
                }
                roder_api::conversation::ConversationItem::ToolResult(m) => {
                    Some(format!("tool: {}", m.result))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[async_trait::async_trait]
impl InferenceEngine for OpenAiResponsesEngine {
    fn id(&self) -> InferenceEngineId {
        "openai-responses".to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: false,
            tool_calls: false,
            parallel_tool_calls: false,
            reasoning_summaries: true,
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
        Ok(vec![
            ModelDescriptor {
                id: "gpt-4o".to_string(),
                name: "GPT-4o".to_string(),
                context_window: None,
            },
            ModelDescriptor {
                id: "gpt-5.5".to_string(),
                name: "GPT-5.5".to_string(),
                context_window: None,
            },
        ])
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let mut body = json!({
            "model": request.model.model,
            "input": Self::input_text(&request),
        });
        if let Some(system) = request.instructions.system {
            body["instructions"] = json!(system);
        }
        if let Some(max_tokens) = request.output.max_tokens {
            body["max_output_tokens"] = json!(max_tokens);
        }
        let response = reqwest::Client::new()
            .post("https://api.openai.com/v1/responses")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "OpenAI Responses error {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            );
        }
        let value: serde_json::Value = response.json().await?;
        let text = value
            .get("output_text")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| extract_output_text(&value))
            .unwrap_or_default();
        let stream = futures::stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta { text })),
            Ok(InferenceEvent::ProviderMetadata(value.clone())),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: value
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                provider_response_id: value.get("id").and_then(|v| v.as_str()).map(str::to_string),
            })),
        ]);
        Ok(Box::pin(stream))
    }
}

fn extract_output_text(value: &serde_json::Value) -> Option<String> {
    let output = value.get("output")?.as_array()?;
    let mut parts = Vec::new();
    for item in output {
        if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
            for block in content {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    parts.push(text.to_string());
                }
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_responses_output_text() {
        let value = serde_json::json!({
            "output": [{ "content": [{ "text": "hello" }, { "text": " world" }] }]
        });
        assert_eq!(extract_output_text(&value).unwrap(), "hello world");
    }
}
