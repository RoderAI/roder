use std::sync::{Arc, Mutex};

use futures::stream;
use roder_api::events::RoderEvent;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    InstructionBundle, MessageDelta, RuntimeProfile,
};
use roder_core::{Runtime, RuntimeConfig, RuntimeReliabilityConfig, StartTurnRequest};

#[tokio::test]
async fn eval_runtime_retries_transient_provider_stream_decode_failure() {
    let calls = Arc::new(Mutex::new(0_u32));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FlakyStreamEngine {
        calls: calls.clone(),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: roder_api::catalog::PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                runtime_profile: RuntimeProfile::Eval,
                reliability: RuntimeReliabilityConfig {
                    provider_retry_max_attempts: 2,
                    provider_retry_initial_backoff_ms: 0,
                    ..RuntimeReliabilityConfig::default()
                },
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut rx = runtime.subscribe_events();

    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-provider-stream-retry".to_string(),
            message: "finish despite one transient stream failure".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: InstructionBundle::default(),
            developer_context: None,
            task_ledger_required: false,
        })
        .await
        .unwrap();

    let mut saw_retry = false;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let envelope = rx.recv().await.unwrap();
            if envelope.turn_id.as_deref() != Some(&turn_id) {
                continue;
            }
            match envelope.event {
                RoderEvent::ReliabilityRetryRecorded(retry) => {
                    saw_retry = true;
                    assert_eq!(retry.attempt, 1);
                    assert_eq!(retry.max_attempts, 2);
                    assert_eq!(retry.context.provider.as_deref(), Some("mock"));
                    assert_eq!(retry.context.model.as_deref(), Some("mock"));
                    assert!(retry.details.message.contains("stream_decode_error"));
                }
                RoderEvent::TurnCompleted(_) => break,
                RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                _ => {}
            }
        }
    })
    .await
    .unwrap();

    assert!(saw_retry);
    assert_eq!(*calls.lock().unwrap(), 2);
}

struct FlakyStreamEngine {
    calls: Arc<Mutex<u32>>,
}

#[async_trait::async_trait]
impl InferenceEngine for FlakyStreamEngine {
    fn id(&self) -> String {
        roder_api::catalog::PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
        Ok(roder_api::catalog::models_for_provider(
            roder_api::catalog::PROVIDER_MOCK,
            true,
        ))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let call_number = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls
        };
        let events: Vec<anyhow::Result<InferenceEvent>> = if call_number == 1 {
            vec![Err(anyhow::anyhow!("error decoding response body"))]
        } else {
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "done after retry".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })),
            ]
        };
        Ok(Box::pin(stream::iter(events)))
    }
}
