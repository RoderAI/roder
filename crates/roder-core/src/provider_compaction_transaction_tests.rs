use super::*;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::transcript::{ToolCallRecord, ToolResultRecord, UserMessage};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy)]
enum Ending {
    Disconnect,
    Failed,
    ConflictingBoundary,
    ToolCall,
    Pending,
}

struct TransactionEngine {
    ending: Ending,
    attempts: AtomicUsize,
    staged: Arc<tokio::sync::Notify>,
}

fn boundary(id: &str) -> InferenceEvent {
    InferenceEvent::ProviderMetadata(json!({
        "output": [{"type":"compaction","id":id,"encrypted_content":id}],
        "compacted_input": [{"type":"compaction","id":id,"encrypted_content":id}]
    }))
}

#[async_trait::async_trait]
impl InferenceEngine for TransactionEngine {
    fn id(&self) -> String {
        "transactional".into()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    fn requires_native_compaction(&self) -> bool {
        true
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
        _: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        panic!("failed native compaction must not silently summarize")
    }

    async fn compact_turn(
        &self,
        _: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<Option<InferenceEventStream>> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.transcript, original_history());
        let mut events = vec![
            Ok(boundary("candidate")),
            Ok(InferenceEvent::Usage(TokenUsage::new(30, 10, 40))),
        ];
        match self.ending {
            Ending::Disconnect => events.push(Err(ProviderFailure::new(
                ProviderFailureKind::StreamInterrupted,
                "disconnected",
            )
            .into())),
            Ending::Failed => events.push(Ok(InferenceEvent::Failed(InferenceFailure {
                message: "failed".into(),
            }))),
            Ending::ConflictingBoundary => {
                events.push(Ok(boundary("conflicting")));
                events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: None,
                    provider_response_id: None,
                })));
            }
            Ending::ToolCall => {
                events.push(Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "invalid".into(),
                    name: "write".into(),
                    arguments: "{}".into(),
                })))
            }
            Ending::Pending => {
                let staged = self.staged.clone();
                return Ok(Some(Box::pin(futures::stream::iter(events).chain(
                    futures::stream::once(async move {
                        staged.notify_one();
                        std::future::pending::<anyhow::Result<InferenceEvent>>().await
                    }),
                ))));
            }
        }
        Ok(Some(Box::pin(futures::stream::iter(events))))
    }
}

fn original_history() -> Vec<TranscriptItem> {
    vec![
        TranscriptItem::UserMessage(UserMessage::text("preserve original goal")),
        TranscriptItem::ToolCall(ToolCallRecord {
            id: "read-proof".into(),
            name: "read_file".into(),
            arguments: r#"{"path":"proof.txt"}"#.into(),
        }),
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "read-proof".into(),
            name: Some("read_file".into()),
            result: "original tool evidence".into(),
            display_payload: None,
            is_error: false,
        }),
    ]
}

fn runtime(dir: &std::path::Path, engine: Arc<TransactionEngine>) -> Runtime {
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(engine);
    registry.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: dir.into(),
    }));
    let mut config = crate::RuntimeConfig::default();
    config.reliability.provider_retry_max_attempts = 2;
    config.reliability.provider_retry_initial_backoff_ms = 0;
    Runtime::new(registry.build().unwrap(), config).unwrap()
}

async fn persist_history(runtime: &Runtime, thread: &ThreadId, turn: &TurnId) {
    for item in original_history() {
        runtime
            .persist_turn_item(thread, turn, &item)
            .await
            .unwrap();
    }
}

async fn assert_replay_and_billing(runtime: &Runtime, thread: &ThreadId, expected_tokens: u64) {
    let snapshot = runtime.load_thread(thread).await.unwrap().unwrap();
    let items: Vec<_> = snapshot
        .turns
        .into_iter()
        .flat_map(|turn| turn.items)
        .collect();
    assert_eq!(
        crate::compaction::trim_to_last_compaction_boundary(items.clone()),
        original_history(),
        "failed compaction must leave replay evidence intact"
    );
    assert!(!items.iter().any(crate::compaction::is_compaction_boundary));
    assert_eq!(
        snapshot.metadata.unwrap().usage.unwrap().total_tokens,
        expected_tokens,
        "usage remains billable even though the candidate was discarded"
    );
}

#[tokio::test]
async fn failed_compaction_preserves_history_and_usage_after_cold_restart() {
    for ending in [
        Ending::Disconnect,
        Ending::Failed,
        Ending::ConflictingBoundary,
        Ending::ToolCall,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(TransactionEngine {
            ending,
            attempts: AtomicUsize::new(0),
            staged: Arc::new(tokio::sync::Notify::new()),
        });
        let live = runtime(dir.path(), engine.clone());
        let thread = live.create_thread(None).await.unwrap().thread_id;
        let turn = "compact-failure".to_string();
        persist_history(&live, &thread, &turn).await;
        let error = live
            .compact_with_provider(
                &thread,
                &turn,
                "transactional",
                "model",
                &original_history(),
                None,
            )
            .await;
        assert!(error.is_err());
        let billed = engine.attempts.load(Ordering::SeqCst) as u64 * 40;
        assert_replay_and_billing(&live, &thread, billed).await;
        drop(live);
        let restarted = runtime(dir.path(), engine);
        assert_replay_and_billing(&restarted, &thread, billed).await;
    }
}

#[tokio::test]
async fn steering_preemption_discards_staged_candidate_but_keeps_observed_usage() {
    let dir = tempfile::tempdir().unwrap();
    let staged = Arc::new(tokio::sync::Notify::new());
    let engine = Arc::new(TransactionEngine {
        ending: Ending::Pending,
        attempts: AtomicUsize::new(0),
        staged: staged.clone(),
    });
    let live = runtime(dir.path(), engine.clone());
    let thread = live.create_thread(None).await.unwrap().thread_id;
    let turn = "preempted-compaction".to_string();
    persist_history(&live, &thread, &turn).await;
    let (sender, receiver) = tokio::sync::watch::channel(0);
    let steer = tokio::spawn(async move {
        staged.notified().await;
        sender.send(1).unwrap();
        sender
    });
    let history = original_history();
    let result = preemptible(
        live.compact_with_provider(&thread, &turn, "transactional", "model", &history, None),
        &mut Some(receiver),
    )
    .await;
    assert!(result.err().unwrap().is::<CompactionPreempted>());
    let _sender = steer.await.unwrap();
    assert_replay_and_billing(&live, &thread, 40).await;
    drop(live);
    assert_replay_and_billing(&runtime(dir.path(), engine), &thread, 40).await;
}
