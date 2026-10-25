use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::{StreamExt, stream};
use roder_api::catalog::{PROVIDER_MOCK, models_for_provider};
use roder_api::events::RoderEvent;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceFailure, InferenceProviderContext,
    InferenceTurnContext, MessageDelta, ModelDescriptor, TokenUsage,
};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;

#[derive(Clone, Copy)]
enum Ending {
    Completed,
    ProviderFailure,
    StreamError,
    Interrupted,
    Continue,
}

struct UsageEngine {
    ending: Ending,
    calls: AtomicUsize,
}

fn usage(prompt: u32, output: u32) -> TokenUsage {
    let mut usage = TokenUsage::new(prompt, output, prompt + output);
    usage.cached_prompt_tokens = 2;
    usage.cache_creation_prompt_tokens = 1;
    usage.cache_hit_rate = Some(2.0 / f64::from(prompt));
    usage.service_tier = Some("priority".to_string());
    usage
}

#[async_trait::async_trait]
impl InferenceEngine for UsageEngine {
    fn id(&self) -> String {
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
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let round = self.calls.fetch_add(1, Ordering::SeqCst);
        let mut events = vec![
            Ok(InferenceEvent::Usage(usage(10, 2))),
            Ok(InferenceEvent::Usage(usage(20, 4))),
        ];
        match self.ending {
            Ending::Completed => {
                events.push(Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "done".to_string(),
                    phase: None,
                })));
                events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })));
            }
            Ending::ProviderFailure => {
                events.push(Ok(InferenceEvent::Failed(InferenceFailure {
                    message: "synthetic provider failure".to_string(),
                })));
            }
            Ending::StreamError => {
                events.push(Err(anyhow::anyhow!("synthetic non-retryable stream error")));
            }
            Ending::Interrupted => {
                return Ok(Box::pin(stream::iter(events).chain(stream::pending())));
            }
            Ending::Continue => {
                if round == 0 {
                    events.push(Ok(InferenceEvent::ProviderMetadata(
                        serde_json::json!({"end_turn":false}),
                    )));
                } else {
                    events = vec![
                        Ok(InferenceEvent::Usage(usage(5, 1))),
                        Ok(InferenceEvent::MessageDelta(MessageDelta {
                            text: "done".to_string(),
                            phase: None,
                        })),
                    ];
                }
                events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })));
            }
        }
        Ok(Box::pin(stream::iter(events)))
    }
}

async fn run(ending: Ending) -> (Option<TokenUsage>, roder_api::thread::ThreadUsageMetadata) {
    let directory = tempfile::tempdir().unwrap();
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(Arc::new(UsageEngine {
        ending,
        calls: AtomicUsize::new(0),
    }));
    registry.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.path().join("threads"),
    }));
    let runtime = Arc::new(
        Runtime::new(
            registry.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let thread = runtime.create_thread(None).await.unwrap();
    let thread_id = thread.thread_id;
    let mut events = runtime.bus.subscribe();
    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: thread_id.clone(),
            message: "exercise accounting".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: directory.path().display().to_string(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        })
        .await
        .unwrap();
    let mut interrupted = false;
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let envelope = events.recv().await.unwrap();
            if envelope.turn_id.as_deref() != Some(turn_id.as_str()) {
                continue;
            }
            if matches!(ending, Ending::Interrupted)
                && !interrupted
                && matches!(&envelope.event, RoderEvent::InferenceEventReceived(event)
                    if matches!(&event.event, InferenceEvent::Usage(usage) if usage.total_tokens == 24))
            {
                interrupted = true;
                runtime
                    .interrupt_turn(thread_id.clone(), turn_id.clone())
                    .await
                    .unwrap();
            }
            let usage = match envelope.event {
                RoderEvent::TurnCompleted(event) => event.usage,
                RoderEvent::TurnFailed(event) => event.usage,
                RoderEvent::TurnInterrupted(event) => event.usage,
                _ => continue,
            };
            let snapshot = runtime.load_thread(&thread_id).await.unwrap().unwrap();
            let metadata = snapshot.metadata.unwrap().usage.unwrap_or_default();
            return (usage, metadata);
        }
    })
    .await
    .expect("terminal event");
    runtime.drain_active_turns(Duration::from_secs(5)).await;
    result
}

#[tokio::test]
async fn cumulative_stream_snapshots_are_counted_once() {
    let (observed, metadata) = run(Ending::Completed).await;
    assert_eq!(observed, Some(usage(20, 4)));
    assert_eq!(metadata.total_tokens, 24);
    assert_eq!(metadata.cached_prompt_tokens, 2);
    assert_eq!(metadata.cache_creation_prompt_tokens, 1);
}

#[tokio::test]
async fn inference_rounds_add_their_final_usage_snapshots() {
    let (observed, metadata) = run(Ending::Continue).await;
    let mut expected = usage(20, 4);
    expected.add_assign(&usage(5, 1));
    assert_eq!(observed, Some(expected));
    assert_eq!(metadata.total_tokens, 30);
    assert_eq!(metadata.cached_prompt_tokens, 4);
    assert_eq!(metadata.cache_creation_prompt_tokens, 2);
}

#[tokio::test]
async fn provider_failure_keeps_usage_in_terminal_event_and_metadata() {
    let (observed, metadata) = run(Ending::ProviderFailure).await;
    assert_eq!(observed, Some(usage(20, 4)));
    assert_eq!(metadata.total_tokens, 24);
}

#[tokio::test]
async fn transport_failure_keeps_usage_in_terminal_event_and_metadata() {
    let (observed, metadata) = run(Ending::StreamError).await;
    assert_eq!(observed, Some(usage(20, 4)));
    assert_eq!(metadata.total_tokens, 24);
}

#[tokio::test]
async fn cancellation_keeps_usage_in_terminal_event_and_metadata() {
    let (observed, metadata) = run(Ending::Interrupted).await;
    assert_eq!(observed, Some(usage(20, 4)));
    assert_eq!(metadata.total_tokens, 24);
}
