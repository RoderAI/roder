use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use roder_api::catalog::PROVIDER_MOCK;
use roder_api::events::{EventEnvelope, ThreadId};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::thread::{
    ThreadItem, ThreadItemDelta, ThreadItemEvent, ThreadItemEventKind, ThreadItemStatus,
    ThreadMetadata, ThreadSnapshot, ThreadStore, ThreadStoreFactory, TurnRecord,
};
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_core::{Runtime, RuntimeConfig, fake_provider::FakeInferenceEngine};
use time::OffsetDateTime;
use tokio::sync::Mutex;

struct CountingThreadStoreFactory {
    store: Arc<CountingThreadStore>,
}

struct CountingThreadStore {
    snapshots: Mutex<HashMap<String, ThreadSnapshot>>,
    load_count: AtomicUsize,
}

impl CountingThreadStoreFactory {
    fn new(snapshots: Vec<ThreadSnapshot>) -> Arc<Self> {
        Arc::new(Self {
            store: Arc::new(CountingThreadStore {
                snapshots: Mutex::new(
                    snapshots
                        .into_iter()
                        .filter_map(|snapshot| {
                            snapshot
                                .metadata
                                .clone()
                                .map(|metadata| (metadata.thread_id, snapshot))
                        })
                        .collect(),
                ),
                load_count: AtomicUsize::new(0),
            }),
        })
    }

    fn load_count(&self) -> usize {
        self.store.load_count.load(Ordering::SeqCst)
    }
}

impl ThreadStoreFactory for CountingThreadStoreFactory {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "counting".to_string()
    }

    fn create(&self) -> Arc<dyn ThreadStore> {
        self.store.clone()
    }
}

#[async_trait::async_trait]
impl ThreadStore for CountingThreadStore {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "counting".to_string()
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        self.snapshots.lock().await.insert(
            metadata.thread_id.clone(),
            ThreadSnapshot {
                metadata: Some(metadata.clone()),
                events: Vec::new(),
                turns: Vec::new(),
                item_events: Vec::new(),
                extension_states: Vec::new(),
            },
        );
        Ok(metadata)
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        Ok(self
            .snapshots
            .lock()
            .await
            .values()
            .filter_map(|snapshot| snapshot.metadata.clone())
            .collect())
    }

    async fn load_thread(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
        self.load_count.fetch_add(1, Ordering::SeqCst);
        Ok(self.snapshots.lock().await.get(thread_id).cloned())
    }

    async fn archive_thread(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        Ok(self.snapshots.lock().await.remove(thread_id).is_some())
    }

    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()> {
        if let Some(snapshot) = self.snapshots.lock().await.get_mut(thread_id) {
            snapshot.events.push(envelope.clone());
        }
        Ok(())
    }

    async fn append_item_event(
        &self,
        thread_id: &ThreadId,
        item_event: &ThreadItemEvent,
    ) -> anyhow::Result<()> {
        if let Some(snapshot) = self.snapshots.lock().await.get_mut(thread_id) {
            snapshot.item_events.push(item_event.clone());
        }
        Ok(())
    }
}

fn runtime_with_counting_thread_store(
    factory: Arc<CountingThreadStoreFactory>,
) -> anyhow::Result<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    let thread_store: Arc<dyn ThreadStoreFactory> = factory;
    builder.thread_store_factory(thread_store);
    Runtime::new(builder.build()?, RuntimeConfig::default())
}

fn counting_thread_snapshot(thread_id: &str) -> ThreadSnapshot {
    let turn_id = format!("{thread_id}-turn");
    ThreadSnapshot {
        metadata: Some(ThreadMetadata {
            thread_id: thread_id.to_string(),
            title: Some(format!("Thread {thread_id}")),
            workspace: std::env::current_dir().unwrap().display().to_string(),
            workspace_id: None,
            root_id: None,
            provider: Some(PROVIDER_MOCK.to_string()),
            model: Some("mock".to_string()),
            selection_mode: None,
            tool_allowlist: Vec::new(),
            developer_instructions: None,
            external_tools: Vec::new(),
            runner_destination: None,
            runner_state: None,
            runner_binding: None,
            parent_thread_id: None,
            forked_from_turn_id: None,
            workspace_fork: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            message_count: 1,
            usage: None,
        }),
        events: Vec::new(),
        turns: vec![TurnRecord {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.clone(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            completed_at: None,
            usage: None,
            finish_reason: None,
            items: vec![
                TranscriptItem::UserMessage(UserMessage::text("one")),
                TranscriptItem::UserMessage(UserMessage::text("two")),
            ],
        }],
        item_events: vec![ThreadItemEvent {
            seq: 7,
            event_id: format!("{turn_id}-item-event-7"),
            thread_id: thread_id.to_string(),
            turn_id,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            event: ThreadItemEventKind::ItemStarted {
                item: ThreadItem::AgentMessage {
                    id: "existing-item".to_string(),
                    text: String::new(),
                    phase: None,
                    status: Some(ThreadItemStatus::InProgress),
                },
            },
        }],
        extension_states: Vec::new(),
    }
}

#[tokio::test]
async fn hydrates_persisted_snapshot_once_per_thread() {
    let factory = CountingThreadStoreFactory::new(vec![counting_thread_snapshot("thread-1")]);
    let runtime = runtime_with_counting_thread_store(factory.clone()).unwrap();
    let thread_id = "thread-1".to_string();
    let turn_id = "thread-1-turn".to_string();

    assert!(
        runtime
            .thread_item_exists(&thread_id, &turn_id, "existing-item")
            .await
            .unwrap()
    );
    assert!(
        !runtime
            .thread_item_exists(&thread_id, &turn_id, "missing-item")
            .await
            .unwrap()
    );
    assert_eq!(
        runtime
            .latest_transcript_item_index(&thread_id, &turn_id)
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        runtime
            .latest_transcript_item_index(&thread_id, &turn_id)
            .await
            .unwrap(),
        Some(1)
    );

    let first_recorded = runtime
        .record_thread_item_event_kind(
            &thread_id,
            &turn_id,
            OffsetDateTime::UNIX_EPOCH,
            ThreadItemEventKind::ItemDelta {
                item_id: "existing-item".to_string(),
                delta: ThreadItemDelta::AgentMessageText {
                    delta: "a".to_string(),
                    phase: None,
                },
            },
        )
        .await
        .unwrap();
    let second_recorded = runtime
        .record_thread_item_event_kind(
            &thread_id,
            &turn_id,
            OffsetDateTime::UNIX_EPOCH,
            ThreadItemEventKind::ItemDelta {
                item_id: "existing-item".to_string(),
                delta: ThreadItemDelta::AgentMessageText {
                    delta: "b".to_string(),
                    phase: None,
                },
            },
        )
        .await
        .unwrap();

    assert_eq!(first_recorded.seq, 8);
    assert_eq!(second_recorded.seq, 9);
    assert_eq!(factory.load_count(), 1);
}

#[tokio::test]
async fn forgets_archived_threads() {
    let factory = CountingThreadStoreFactory::new(vec![counting_thread_snapshot("thread-1")]);
    let runtime = runtime_with_counting_thread_store(factory.clone()).unwrap();
    let thread_id = "thread-1".to_string();
    let turn_id = "thread-1-turn".to_string();

    assert!(
        runtime
            .thread_item_exists(&thread_id, &turn_id, "existing-item")
            .await
            .unwrap()
    );
    assert!(runtime.archive_thread(&thread_id).await.unwrap());
    assert!(
        !runtime
            .thread_item_exists(&thread_id, &turn_id, "existing-item")
            .await
            .unwrap()
    );
    assert_eq!(factory.load_count(), 2);
}

#[tokio::test]
async fn evicts_least_recently_used_threads() {
    let cache_capacity = 256;
    let snapshots = (0..=cache_capacity)
        .map(|index| counting_thread_snapshot(&format!("thread-{index}")))
        .collect::<Vec<_>>();
    let factory = CountingThreadStoreFactory::new(snapshots);
    let runtime = runtime_with_counting_thread_store(factory.clone()).unwrap();

    for index in 0..=cache_capacity {
        let thread_id = format!("thread-{index}");
        let turn_id = format!("{thread_id}-turn");
        assert!(
            runtime
                .thread_item_exists(&thread_id, &turn_id, "existing-item")
                .await
                .unwrap()
        );
    }
    let first_pass_loads = factory.load_count();

    assert!(
        runtime
            .thread_item_exists(
                &"thread-0".to_string(),
                &"thread-0-turn".to_string(),
                "existing-item"
            )
            .await
            .unwrap()
    );
    assert_eq!(factory.load_count(), first_pass_loads + 1);
}
