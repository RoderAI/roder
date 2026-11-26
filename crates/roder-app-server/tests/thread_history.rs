use super::*;
use roder_api::extension_state::ExtensionStateRecord;
use roder_api::lifecycle::{TurnCleanupState, TurnLifecycleRecord, TurnLifecycleState};
use roder_api::thread::{
    ThreadHistoryPage, ThreadHistoryReadOptions, ThreadHistoryTurn, ThreadHistoryTurnStatus,
    ThreadItem,
};
use roder_protocol::{ThreadHistoryReadResult, ThreadResumeResult, TurnStartResult};

#[derive(Default)]
struct HistoryOnlyStore {
    lifecycle: tokio::sync::Mutex<Vec<ExtensionStateRecord>>,
}

struct HistoryOnlyFactory(Arc<HistoryOnlyStore>);

impl ThreadStoreFactory for HistoryOnlyFactory {
    fn id(&self) -> String {
        "history-only".into()
    }

    fn create(&self) -> Arc<dyn ThreadStore> {
        self.0.clone()
    }
}

#[async_trait::async_trait]
impl ThreadStore for HistoryOnlyStore {
    fn id(&self) -> String {
        "history-only".into()
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        Ok(metadata)
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        Ok(Vec::new())
    }

    async fn load_thread(&self, thread_id: &String) -> anyhow::Result<Option<ThreadSnapshot>> {
        // The inference worker may hydrate its context after accepting a turn.
        if thread_id == "metadata-only" && tokio::task::try_id().is_some() {
            return Ok(None);
        }
        panic!("bounded history and workspace admission must never load a complete snapshot")
    }

    async fn load_thread_metadata(
        &self,
        thread_id: &String,
    ) -> anyhow::Result<Option<ThreadMetadata>> {
        FailingThreadStore.load_thread_metadata(thread_id).await
    }

    async fn load_extension_states(&self, _: &String) -> anyhow::Result<Vec<ExtensionStateRecord>> {
        Ok(self.lifecycle.lock().await.clone())
    }

    async fn append_extension_state(
        &self,
        _: &String,
        record: &ExtensionStateRecord,
    ) -> anyhow::Result<()> {
        self.lifecycle.lock().await.push(record.clone());
        Ok(())
    }

    async fn read_history_page(
        &self,
        thread_id: &String,
        options: ThreadHistoryReadOptions,
    ) -> anyhow::Result<Option<ThreadHistoryPage>> {
        if thread_id == "missing" {
            return Ok(None);
        }
        if thread_id == "unsupported" {
            anyhow::bail!("bounded history unsupported");
        }
        let end = match options.before.as_deref() {
            None => 500_usize,
            Some("older-340") => 340,
            Some(_) => anyhow::bail!("invalid history cursor"),
        };
        assert!(options.limit <= 200);
        assert!(options.max_bytes <= 2 * 1024 * 1024);
        let start = end.saturating_sub(options.limit);
        let metadata = FailingThreadStore
            .load_thread_metadata(thread_id)
            .await?
            .unwrap();
        Ok(Some(ThreadHistoryPage {
            metadata,
            turns: vec![ThreadHistoryTurn {
                turn_id: "turn".into(),
                created_at: time::OffsetDateTime::UNIX_EPOCH,
                completed_at: Some(time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(2)),
                status: ThreadHistoryTurnStatus::Failed,
                error: Some("fixture failure".into()),
                usage: None,
                finish_reason: Some("length".into()),
                items: (start..end)
                    .map(|index| ThreadItem::AgentMessage {
                        id: format!("item-{index}"),
                        text: index.to_string(),
                        phase: None,
                        status: None,
                    })
                    .collect(),
            }],
            before: (start > 0).then(|| format!("older-{start}")),
            has_more: start > 0,
        }))
    }

    async fn append_event(
        &self,
        _: &String,
        _: &roder_api::events::EventEnvelope,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

fn history_client() -> LocalAppClient {
    history_client_with_store(Arc::new(HistoryOnlyStore::default()))
}

fn history_client_with_store(store: Arc<HistoryOnlyStore>) -> LocalAppClient {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(HistoryOnlyFactory(store)));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    LocalAppClient::new(Arc::new(app_server(runtime)))
}

#[tokio::test]
async fn history_endpoint_pages_without_loading_snapshot() {
    let client = history_client();
    let latest: ThreadHistoryReadResult = request(
        &client,
        "thread/history/read",
        Some(serde_json::json!({"threadId":"history"})),
    )
    .await;
    assert_eq!(latest.before.as_deref(), Some("older-340"));
    assert!(latest.has_more);
    let turns = latest.thread.unwrap().turns.unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].items.len(), 160);
    assert_eq!(
        serde_json::to_value(&turns[0].items[0]).unwrap()["id"],
        "item-340"
    );
    assert_eq!(
        serde_json::to_value(&turns[0].items[159]).unwrap()["id"],
        "item-499"
    );
    assert_eq!(turns[0].status, "failed");
    assert_eq!(turns[0].error, Some(serde_json::json!("fixture failure")));
    assert_eq!(turns[0].duration_ms, Some(2000));
    assert_eq!(turns[0].finish_reason.as_deref(), Some("length"));

    let older: ThreadHistoryReadResult = request(
        &client,
        "thread/history/read",
        Some(serde_json::json!({"threadId":"history","before":latest.before})),
    )
    .await;
    let turns = older.thread.unwrap().turns.unwrap();
    assert_eq!(turns[0].items.len(), 120);
    assert_eq!(
        serde_json::to_value(&turns[0].items[0]).unwrap()["id"],
        "item-220"
    );
    assert_eq!(
        serde_json::to_value(&turns[0].items[119]).unwrap()["id"],
        "item-339"
    );
}

#[tokio::test]
async fn bounded_thread_resume_reconciles_lifecycle_and_publishes_thread_loaded() {
    let thread_id = "history".to_string();
    let turn_id = "stale-turn".to_string();
    let store = Arc::new(HistoryOnlyStore::default());
    let record = TurnLifecycleRecord::new(
        thread_id.clone(),
        turn_id.clone(),
        TurnLifecycleState::Running,
        TurnCleanupState::NotRequested,
        None,
        time::OffsetDateTime::UNIX_EPOCH,
    );
    store
        .append_extension_state(&thread_id, &record.extension_state().unwrap())
        .await
        .unwrap();
    let client = history_client_with_store(store.clone());
    let mut events = client.subscribe_events();
    let read: ThreadHistoryReadResult = request(
        &client,
        "thread/history/read",
        Some(serde_json::json!({"threadId": thread_id, "limit": 7})),
    )
    .await;
    assert_eq!(read.thread.unwrap().turns.unwrap()[0].items.len(), 7);
    assert!(events.try_recv().is_err());
    assert_eq!(store.lifecycle.lock().await.len(), 1);

    let resumed: ThreadResumeResult = request(
        &client,
        "thread/resume",
        Some(serde_json::json!({"threadId": thread_id, "limit": 7})),
    )
    .await;
    assert_eq!(
        resumed.history.thread.unwrap().turns.unwrap()[0]
            .items
            .len(),
        7
    );
    assert_eq!(resumed.history.before.as_deref(), Some("older-493"));
    assert_eq!(resumed.lifecycle.records.len(), 1);
    assert_eq!(resumed.lifecycle.records[0].turn_id, turn_id);
    assert_eq!(
        resumed.lifecycle.records[0].state,
        TurnLifecycleState::RecoveryNeeded
    );
    assert!(std::iter::from_fn(|| events.try_recv().ok()).any(|envelope| {
        matches!(envelope.event, roder_api::events::RoderEvent::ThreadLoaded(event) if event.thread_id == thread_id)
    }));
}

#[tokio::test]
async fn turn_start_resolves_workspace_without_a_full_thread_read() {
    let client = history_client();
    let result: TurnStartResult = request(
        &client,
        "turn/start",
        Some(serde_json::json!({"threadId": "metadata-only", "input": [{"type": "text", "text": "hi"}]})),
    ).await;
    assert!(!result.turn_id.is_empty());
}

#[tokio::test]
async fn history_endpoint_clamps_budgets_and_returns_null_for_unknown_threads() {
    let client = history_client();
    let page: ThreadHistoryReadResult = request(
        &client,
        "thread/history/read",
        Some(serde_json::json!({"threadId":"history","limit":9999,"maxBytes":99999999})),
    )
    .await;
    assert_eq!(page.thread.unwrap().turns.unwrap()[0].items.len(), 200);
    let page: ThreadHistoryReadResult = request(
        &client,
        "thread/history/read",
        Some(serde_json::json!({"threadId":"missing"})),
    )
    .await;
    assert!(page.thread.is_none());
    assert!(page.before.is_none());
    assert!(!page.has_more);
}

#[tokio::test]
async fn history_endpoint_surfaces_store_and_cursor_errors() {
    let client = history_client();
    for params in [
        serde_json::json!({"threadId":"unsupported"}),
        serde_json::json!({"threadId":"history","before":"invalid"}),
    ] {
        let response = client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(serde_json::json!(1)),
                method: "thread/history/read".into(),
                params: Some(params),
            })
            .await;
        assert!(response.error.is_some());
        assert!(response.result.is_none());
    }
}

#[tokio::test]
async fn history_endpoint_traverses_jsonl_pages_in_chronological_order() {
    use roder_api::thread::{ThreadItemEvent, ThreadItemEventKind};

    let root = std::env::temp_dir().join(format!("history-rpc-{}", uuid::Uuid::new_v4()));
    let factory = Arc::new(JsonlThreadStoreFactory {
        base_path: root.clone(),
    });
    let store = factory.create();
    let thread_id = "indexed-history".to_string();
    let metadata = FailingThreadStore
        .load_thread_metadata(&thread_id)
        .await
        .unwrap()
        .unwrap();
    store.create_thread(metadata).await.unwrap();
    for index in 0..41_u64 {
        store
            .append_item_event(
                &thread_id,
                &ThreadItemEvent {
                    seq: index + 1,
                    event_id: format!("event-{index}"),
                    thread_id: thread_id.clone(),
                    turn_id: "event-only-turn".into(),
                    timestamp: time::OffsetDateTime::UNIX_EPOCH
                        + time::Duration::seconds(index as i64),
                    event: ThreadItemEventKind::ItemCompleted {
                        item: ThreadItem::AgentMessage {
                            id: format!("item-{index}"),
                            text: index.to_string(),
                            phase: None,
                            status: None,
                        },
                    },
                },
            )
            .await
            .unwrap();
    }
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(factory);
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let client = LocalAppClient::new(Arc::new(app_server(runtime)));
    let mut before = None::<String>;
    let mut all = Vec::new();
    loop {
        let page: ThreadHistoryReadResult = request(
            &client,
            "thread/history/read",
            Some(serde_json::json!({"threadId":thread_id,"limit":7,"before":before})),
        )
        .await;
        let turns = page.thread.unwrap().turns.unwrap();
        let mut ids = turns
            .iter()
            .flat_map(|turn| &turn.items)
            .map(|item| {
                serde_json::to_value(item).unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert!(!ids.is_empty());
        assert!(ids.len() <= 7);
        ids.append(&mut all);
        all = ids;
        if !page.has_more {
            assert!(page.before.is_none());
            break;
        }
        assert_ne!(page.before, before);
        before = page.before;
    }
    assert_eq!(
        all,
        (0..41)
            .map(|index| format!("item-{index}"))
            .collect::<Vec<_>>()
    );
    std::fs::remove_dir_all(root).unwrap();
}
