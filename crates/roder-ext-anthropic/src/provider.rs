use futures::stream;
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor,
};

pub struct AnthropicEngine {
    _api_key: String,
}

impl AnthropicEngine {
    pub fn new(api_key: String) -> Self {
        Self { _api_key: api_key }
    }
}

#[async_trait::async_trait]
impl InferenceEngine for AnthropicEngine {
    fn id(&self) -> InferenceEngineId {
        "anthropic".to_string()
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
                id: "claude-3-5-sonnet-20241022".to_string(),
                name: "Claude 3.5 Sonnet".to_string(),
            },
        ])
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let stream = stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "Stub response from Anthropic".to_string(),
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
            })),
        ]);

        Ok(Box::pin(stream))
    }
}