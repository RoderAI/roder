#[cfg(unix)]
use roder_api::events::{EventEnvelope, RoderEvent, TranscriptItemAppended};
use roder_api::thread::{ThreadMetadata, ThreadStore};
#[cfg(unix)]
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_jsonl_thread_store::store::JsonlThreadStore;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::Barrier;

#[tokio::test]
async fn independent_store_instances_preserve_and_repair_index_entries() {
    const COUNT: usize = 32;
    let workspace =
        std::env::temp_dir().join(format!("roder-audit-index-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&workspace).unwrap();
    let root = workspace.join("threads");
    let barrier = Arc::new(Barrier::new(COUNT));
    let mut creates = tokio::task::JoinSet::new();
    for i in 0..COUNT {
        let store = JsonlThreadStore {
            base_path: root.clone(),
        };
        let workspace = workspace.display().to_string();
        let barrier = barrier.clone();
        creates.spawn(async move {
            let now = OffsetDateTime::now_utc();
            barrier.wait().await;
            store
                .create_thread(ThreadMetadata {
                    thread_id: format!("thread-{i}"),
                    title: None,
                    workspace,
                    workspace_id: None,
                    root_id: None,
                    provider: None,
                    model: None,
                    selection_mode: None,
                    tool_allowlist: Vec::new(),
                    service_tier: None,
                    developer_instructions: None,
                    external_tools: Vec::new(),
                    runner_destination: None,
                    runner_state: None,
                    runner_binding: None,
                    parent_thread_id: None,
                    forked_from_turn_id: None,
                    workspace_fork: None,
                    created_at: now,
                    updated_at: now,
                    message_count: 0,
                    usage: None,
                })
                .await
                .unwrap();
        });
    }
    while let Some(result) = creates.join_next().await {
        result.unwrap();
    }
    let store = JsonlThreadStore { base_path: root };
    assert_eq!(store.list_threads().await.unwrap().len(), COUNT);

    // An old index may parse cleanly even though concurrent writers lost entries.
    let index_path = store.base_path.join("thread-index.json");
    let mut index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&index_path).unwrap()).unwrap();
    index.as_object_mut().unwrap().remove("version");
    index["threads"].as_array_mut().unwrap().truncate(1);
    std::fs::write(&index_path, serde_json::to_vec(&index).unwrap()).unwrap();

    assert_eq!(store.list_threads().await.unwrap().len(), COUNT);

    // Simulate a process exit between updating metadata and installing the index.
    let mut index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&index_path).unwrap()).unwrap();
    index["threads"].as_array_mut().unwrap().truncate(1);
    std::fs::write(&index_path, serde_json::to_vec(&index).unwrap()).unwrap();
    let dirty_path = store.base_path.join("thread-index.dirty");
    std::fs::write(&dirty_path, []).unwrap();
    assert_eq!(store.list_threads().await.unwrap().len(), COUNT);
    assert!(!dirty_path.exists());

    assert!(store.archive_thread(&"thread-0".into()).await.unwrap());
    assert_eq!(store.list_threads().await.unwrap().len(), COUNT - 1);

    #[cfg(unix)]
    assert_late_metadata_does_not_restore_archived_thread(&store, &workspace).await;

    std::fs::remove_dir_all(workspace).unwrap();
}

#[cfg(unix)]
async fn assert_late_metadata_does_not_restore_archived_thread(
    store: &JsonlThreadStore,
    workspace: &std::path::Path,
) {
    use std::time::Duration;

    let index_lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(store.base_path.join("thread-index.lock"))
        .unwrap();
    index_lock.try_lock().unwrap();
    let active = store.base_path.join("thread-1");
    let archived = workspace.join("archived_threads").join("thread-1");
    let now = OffsetDateTime::now_utc();
    let event = RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
        thread_id: "thread-1".into(),
        turn_id: "turn-1".into(),
        item_type: "user_message".into(),
        item_index: Some(0),
        item: Some(TranscriptItem::UserMessage(UserMessage::text(
            "last prompt",
        ))),
        timestamp: now,
    });
    let envelope = EventEnvelope {
        event_id: uuid::Uuid::new_v4().to_string(),
        seq: 1,
        timestamp: now,
        source: event.source(),
        kind: event.kind().into(),
        thread_id: event.thread_id().cloned(),
        turn_id: event.turn_id().cloned(),
        event,
    };
    let other = JsonlThreadStore {
        base_path: store.base_path.clone(),
    };
    let writer =
        tokio::spawn(async move { other.append_event(&"thread-1".into(), &envelope).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !std::fs::read(active.join("events.jsonl")).is_ok_and(|bytes| bytes.ends_with(b"\n"))
        {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    // The archive transaction owns this lock while moving the active directory.
    std::fs::rename(&active, &archived).unwrap();
    drop(index_lock);
    assert!(
        writer
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("archived")
    );
    assert!(!active.exists());
    assert!(archived.join("events.jsonl").is_file());
    assert!(
        !store
            .list_threads()
            .await
            .unwrap()
            .iter()
            .any(|thread| thread.thread_id == "thread-1")
    );
}
