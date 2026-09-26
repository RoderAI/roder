use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{StreamExt, stream};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::*;
use roder_api::transcript::TranscriptItem;
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use serde_json::json;

struct HistoryEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
    wait_for_steer: bool,
}

#[async_trait::async_trait]
impl InferenceEngine for HistoryEngine {
    fn id(&self) -> String {
        "mock".into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }
    async fn list_models(
        &self,
        _: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(vec![])
    }
    async fn stream_turn(
        &self,
        _: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let attempt = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        if attempt == 1 {
            let events = vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "completed commentary".into(),
                    phase: Some("commentary".into()),
                })),
                Ok(InferenceEvent::OutputItemCompleted(
                    json!({"id":"message1","type":"message","role":"assistant","phase":"commentary",
                    "content":[{"type":"output_text","text":"completed commentary"}]}),
                )),
                Ok(InferenceEvent::ReasoningDelta(ReasoningDelta {
                    text: "completed reasoning".into(),
                })),
                Ok(InferenceEvent::OutputItemCompleted(
                    json!({"id":"reason1","type":"reasoning","encrypted_content":"opaque",
                    "summary":[{"type":"summary_text","text":"completed reasoning"}]}),
                )),
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "unfinished text".into(),
                    phase: None,
                })),
            ];
            if self.wait_for_steer {
                return Ok(Box::pin(stream::iter(events).chain(stream::pending())));
            }
            return Ok(Box::pin(stream::iter(events).chain(stream::once(async {
                Err(roder_api::provider_error::ProviderFailure::new(
                    roder_api::provider_error::ProviderFailureKind::StreamInterrupted,
                    "fixture disconnect",
                )
                .into())
            }))));
        }
        Ok(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "final".into(),
                phase: None,
            })),
            Ok(InferenceEvent::OutputItemCompleted(
                json!({"id":"message2","type":"message","role":"assistant",
                "content":[{"type":"output_text","text":"final"}]}),
            )),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("completed".into()),
                provider_response_id: None,
            })),
        ])))
    }
}

async fn exercise(wait_for_steer: bool) {
    let directory = tempfile::tempdir().unwrap();
    let engine = Arc::new(HistoryEngine {
        requests: Mutex::new(vec![]),
        wait_for_steer,
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.path().to_path_buf(),
    }));
    let mut config = RuntimeConfig::default();
    config.reliability.provider_retry_initial_backoff_ms = 0;
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), config).unwrap());
    let thread = runtime.create_thread(None).await.unwrap().thread_id;
    let mut events = runtime.subscribe_events();
    let turn = runtime
        .start_turn(StartTurnRequest {
            thread_id: thread.clone(),
            message: "go".into(),
            images: vec![],
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: directory.path().to_string_lossy().into(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        })
        .await
        .unwrap();
    let mut steered = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        if wait_for_steer
            && !steered
            && event.kind == "inference.event_received"
            && serde_json::to_string(&event)
                .unwrap()
                .contains("unfinished text")
        {
            runtime
                .steer_turn(
                    thread.clone(),
                    turn.clone(),
                    "new constraint".into(),
                    vec![],
                )
                .await
                .unwrap();
            steered = true;
        }
        assert_ne!(event.kind, "turn.failed", "{event:?}");
        if event.kind == "turn.completed" {
            break;
        }
    }
    {
        let requests = engine.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let replay = &requests[1].transcript;
        assert_eq!(replay.iter().filter(|item| matches!(item, TranscriptItem::AssistantMessage(message) if message.text == "completed commentary")).count(), 1);
        assert_eq!(replay.iter().filter(|item| matches!(item, TranscriptItem::ReasoningSummary(summary) if summary.text == "completed reasoning")).count(), 1);
        assert!(replay.iter().any(|item| matches!(item, TranscriptItem::ProviderMetadata(metadata) if metadata.to_string().contains("opaque"))));
        assert!(
            !serde_json::to_string(replay)
                .unwrap()
                .contains("unfinished text")
        );
        if wait_for_steer {
            assert!(
                serde_json::to_string(replay)
                    .unwrap()
                    .contains("new constraint")
            );
        }
    }
    let snapshot = runtime.load_thread(&thread).await.unwrap().unwrap();
    let items = &snapshot
        .turns
        .iter()
        .find(|entry| entry.turn_id == turn)
        .unwrap()
        .items;
    assert_eq!(items.iter().filter(|item| matches!(item, TranscriptItem::AssistantMessage(message) if message.text == "final")).count(), 1);
    assert_eq!(items.iter().filter(|item| matches!(item, TranscriptItem::AssistantMessage(message) if message.text == "completed commentary")).count(), 1);
    assert!(
        !serde_json::to_string(items)
            .unwrap()
            .contains("unfinished text")
    );
}

#[tokio::test]
async fn completed_output_survives_disconnect_without_unfinished_deltas() {
    exercise(false).await;
}

#[tokio::test]
async fn completed_output_survives_steering_without_duplicates() {
    exercise(true).await;
}
