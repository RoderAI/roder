use std::pin::Pin;
use futures::{Stream, stream};
use roder_api::inference::*;
use roder_api::extension::InferenceEngineId;

pub struct FakeInferenceEngine;

#[async_trait::async_trait]
impl InferenceEngine for FakeInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        "fake-provider".to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            supports_tools: false,
            supports_vision: false,
            supports_reasoning: false,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(vec![ModelDescriptor {
            id: "fake-model".to_string(),
            name: "Fake Model".to_string(),
        }])
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let stream = stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta { text: "hello".to_string() })),
            Ok(InferenceEvent::MessageDelta(MessageDelta { text: " from".to_string() })),
            Ok(InferenceEvent::MessageDelta(MessageDelta { text: " roder".to_string() })),
            Ok(InferenceEvent::Completed(CompletionMetadata { stop_reason: Some("stop".to_string()) })),
        ]);

        Ok(Box::pin(stream))
    }
}
