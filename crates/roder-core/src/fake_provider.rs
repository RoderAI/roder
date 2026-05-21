use futures::stream;
use roder_api::catalog::{PROVIDER_MOCK, models_for_provider};
use roder_api::conversation::ConversationItem;
use roder_api::extension::InferenceEngineId;
use roder_api::inference::*;

pub struct FakeInferenceEngine;

#[async_trait::async_trait]
impl InferenceEngine for FakeInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_MOCK, true))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        if should_request_user_input(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-user-input".to_string(),
                    name: "request_user_input".to_string(),
                    arguments: serde_json::json!({
                        "questions": [{
                            "header": "Choice",
                            "id": "choice",
                            "question": "Which option should be used?",
                            "options": [
                                { "label": "A", "description": "Use option A." },
                                { "label": "B", "description": "Use option B." }
                            ]
                        }]
                    })
                    .to_string(),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if user_input_unavailable(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::Failed(InferenceFailure {
                message: "clarification unavailable in non-interactive runtime profile".to_string(),
            }))]);
            return Ok(Box::pin(stream));
        }
        let stream = stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "hello".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: " from".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: " roder".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ]);

        Ok(Box::pin(stream))
    }
}

fn should_request_user_input(request: &AgentInferenceRequest) -> bool {
    request.conversation.iter().any(|item| {
        matches!(
            item,
            ConversationItem::UserMessage(message)
                if message.text.contains("FAKE_REQUEST_USER_INPUT")
        )
    }) && !request.conversation.iter().any(|item| {
        matches!(
            item,
            ConversationItem::ToolResult(result)
                if result.name.as_deref() == Some("request_user_input")
        )
    })
}

fn user_input_unavailable(request: &AgentInferenceRequest) -> bool {
    request.conversation.iter().any(|item| {
        matches!(
            item,
            ConversationItem::ToolResult(result)
                if result.name.as_deref() == Some("request_user_input")
                    && result.is_error
                    && result.result.contains("User input is unavailable")
        )
    })
}
