use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use futures::stream;
use roder_api::artifacts::ContextArtifactStore;
use roder_api::catalog::{PROVIDER_MOCK, models_for_provider};
use roder_api::events::{EventEnvelope, RoderEvent, ThreadId};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::extension_state::ExtensionStateRecord;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    ModelDescriptor, TokenUsage,
};
use roder_api::lifecycle::TurnLifecycleState;
use roder_api::thread::{
    ThreadItemEvent, ThreadMetadata, ThreadSnapshot, ThreadStore, ThreadStoreFactory,
};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use tokio::sync::Notify;

struct CommitFault {
    injected: AtomicBool,
    finished: AtomicBool,
    committed: Notify,
    interrupt_recorded: Notify,
    release: Notify,
    fail_after_commit: bool,
    usage_writes: AtomicUsize,
    returned_errors: AtomicUsize,
}

impl CommitFault {
    fn new(fail_after_commit: bool) -> Self {
        Self {
            injected: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            committed: Notify::new(),
            interrupt_recorded: Notify::new(),
            release: Notify::new(),
            fail_after_commit,
            usage_writes: AtomicUsize::new(0),
            returned_errors: AtomicUsize::new(0),
        }
    }
}

struct FaultingStoreFactory {
    store: Arc<FaultingStore>,
}

impl ThreadStoreFactory for FaultingStoreFactory {
    fn id(&self) -> String {
        self.store.id()
    }

    fn create(&self) -> Arc<dyn ThreadStore> {
        self.store.clone()
    }
}

struct FaultingStore {
    delegate: Arc<dyn ThreadStore>,
    fault: Arc<CommitFault>,
}

#[async_trait::async_trait]
impl ThreadStore for FaultingStore {
    fn id(&self) -> String {
        self.delegate.id()
    }

    fn local_thread_root(&self) -> Option<PathBuf> {
        self.delegate.local_thread_root()
    }

    fn context_artifact_store(&self) -> Option<ContextArtifactStore> {
        self.delegate.context_artifact_store()
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        self.delegate.create_thread(metadata).await
    }

    async fn update_thread_metadata(
        &self,
        metadata: ThreadMetadata,
    ) -> anyhow::Result<ThreadMetadata> {
        let has_terminal_usage = metadata
            .usage
            .as_ref()
            .is_some_and(|usage| !usage.accounted_turn_ids.is_empty());
        if !has_terminal_usage {
            return self.delegate.update_thread_metadata(metadata).await;
        }
        self.fault.usage_writes.fetch_add(1, Ordering::SeqCst);
        let committed = self.delegate.update_thread_metadata(metadata).await?;
        if !self.fault.injected.swap(true, Ordering::SeqCst) {
            self.fault.committed.notify_one();
            // Simulate an index update waiting after the metadata file committed.
            self.fault.release.notified().await;
            self.fault.finished.store(true, Ordering::SeqCst);
            if self.fault.fail_after_commit {
                self.fault.returned_errors.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("injected index failure after durable metadata write");
            }
        }
        Ok(committed)
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        self.delegate.list_threads().await
    }

    async fn load_thread_metadata(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Option<ThreadMetadata>> {
        self.delegate.load_thread_metadata(thread_id).await
    }

    async fn load_thread(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
        self.delegate.load_thread(thread_id).await
    }

    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()> {
        self.delegate.append_event(thread_id, envelope).await?;
        if matches!(&envelope.event, RoderEvent::TurnLifecycleUpdated(record)
            if record.state == TurnLifecycleState::InterruptRequested)
        {
            self.fault.interrupt_recorded.notify_one();
        }
        Ok(())
    }

    async fn append_item_event(
        &self,
        thread_id: &ThreadId,
        event: &ThreadItemEvent,
    ) -> anyhow::Result<()> {
        self.delegate.append_item_event(thread_id, event).await
    }

    async fn append_extension_state(
        &self,
        thread_id: &ThreadId,
        record: &ExtensionStateRecord,
    ) -> anyhow::Result<()> {
        self.delegate
            .append_extension_state(thread_id, record)
            .await
    }
}

fn observed_usage() -> TokenUsage {
    let mut usage = TokenUsage::new(20, 4, 24);
    usage.cached_prompt_tokens = 2;
    usage.cache_creation_prompt_tokens = 1;
    usage.cache_hit_rate = Some(0.1);
    usage.service_tier = Some("priority".into());
    usage
}

struct UsageEngine;

#[async_trait::async_trait]
impl InferenceEngine for UsageEngine {
    fn id(&self) -> String {
        PROVIDER_MOCK.into()
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
        Ok(Box::pin(stream::iter([
            Ok(InferenceEvent::Usage(observed_usage())),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".into()),
                provider_response_id: None,
            })),
        ])))
    }
}

async fn interrupt_after_metadata_commit(fail_after_commit: bool) {
    let directory = tempfile::tempdir().unwrap();
    let delegate = JsonlThreadStoreFactory {
        base_path: directory.path().join("threads"),
    }
    .create();
    let fault = Arc::new(CommitFault::new(fail_after_commit));
    let store = Arc::new(FaultingStore {
        delegate: delegate.clone(),
        fault: fault.clone(),
    });
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(Arc::new(UsageEngine));
    registry.thread_store_factory(Arc::new(FaultingStoreFactory { store }));
    let runtime = Arc::new(
        Runtime::new(
            registry.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.into(),
                default_model: "mock".into(),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let thread_id = runtime.create_thread(None).await.unwrap().thread_id;
    let mut events = runtime.bus.subscribe();
    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: thread_id.clone(),
            message: "persist observed usage".into(),
            images: vec![],
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

    tokio::time::timeout(Duration::from_secs(10), fault.committed.notified())
        .await
        .expect("terminal metadata was durably committed");
    let committed = delegate
        .load_thread_metadata(&thread_id)
        .await
        .unwrap()
        .unwrap()
        .usage
        .unwrap();
    assert_eq!(committed.total_tokens, 24);
    assert!(committed.accounted_turn_ids.contains(&turn_id));

    let interrupt = {
        let runtime = runtime.clone();
        let thread_id = thread_id.clone();
        let turn_id = turn_id.clone();
        tokio::spawn(async move { runtime.interrupt_turn(thread_id, turn_id).await })
    };
    tokio::time::timeout(Duration::from_secs(10), fault.interrupt_recorded.notified())
        .await
        .expect("interrupt request persisted while post-commit metadata work remained blocked");
    // The interrupting task now aborts the turn and waits on its accounting lock.
    tokio::task::yield_now().await;
    assert!(!interrupt.is_finished());
    fault.release.notify_one();
    let terminal = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.unwrap();
            if let RoderEvent::TurnInterrupted(event) = event.event
                && event.turn_id == turn_id
            {
                break event;
            }
        }
    })
    .await
    .expect("interrupted terminal event after metadata finalization");
    interrupt.await.unwrap().unwrap();
    runtime.drain_active_turns(Duration::from_secs(5)).await;

    assert_eq!(terminal.usage, Some(observed_usage()));
    let persisted = delegate.load_thread(&thread_id).await.unwrap().unwrap();
    let usage = persisted.metadata.unwrap().usage.unwrap();
    assert_eq!(usage.prompt_tokens, 20);
    assert_eq!(usage.completion_tokens, 4);
    assert_eq!(
        usage.total_tokens, 24,
        "committed usage must not be charged twice"
    );
    assert_eq!(usage.cached_prompt_tokens, 2);
    assert_eq!(usage.cache_creation_prompt_tokens, 1);
    assert_eq!(usage.accounted_turn_ids.len(), 1);
    assert!(usage.accounted_turn_ids.contains(&turn_id));
    assert_eq!(fault.usage_writes.load(Ordering::SeqCst), 1);
    assert!(
        fault.finished.load(Ordering::SeqCst),
        "metadata job survived turn cancellation"
    );
    assert_eq!(
        fault.returned_errors.load(Ordering::SeqCst),
        usize::from(fail_after_commit)
    );
    let terminal_events = persisted
        .events
        .iter()
        .filter(|event| {
            event.turn_id.as_ref() == Some(&turn_id)
                && matches!(
                    event.event,
                    RoderEvent::TurnCompleted(_)
                        | RoderEvent::TurnFailed(_)
                        | RoderEvent::TurnInterrupted(_)
                )
        })
        .count();
    assert_eq!(terminal_events, 1);
}

#[tokio::test]
async fn cancellation_after_metadata_commit_does_not_double_charge() {
    interrupt_after_metadata_commit(false).await;
}

#[tokio::test]
async fn interrupted_retry_after_durable_commit_error_does_not_double_charge() {
    interrupt_after_metadata_commit(true).await;
}
