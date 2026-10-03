use roder_api::{
    artifacts::{ContextArtifactKind, CreateArtifactRequest},
    events::EventEnvelope,
    extension_state::{ExtensionStateRecord, ExtensionStoreScope},
    thread::{ThreadItemEvent, ThreadMetadata, ThreadStore},
};
use roder_ext_mysql_session::{
    MysqlSessionConfig, MysqlSessionStore, ownership::RuntimeOwnerClaim,
};
use std::time::Duration;
use uuid::Uuid;

async fn store() -> MysqlSessionStore {
    let url = std::env::var("RODER_MYSQL_TEST_URL").expect("isolated MySQL test URL required");
    MysqlSessionStore::connect(
        &MysqlSessionConfig::new(url, format!("write-fence-{}", Uuid::new_v4())).unwrap(),
    )
    .await
    .unwrap()
}
fn metadata() -> ThreadMetadata {
    serde_json::from_value(serde_json::json!({
        "thread_id":"thread", "workspace":"/tmp/owner-test", "provider":"test", "model":"test",
        "created_at":"2026-10-03T00:00:00Z", "updated_at":"2026-10-03T00:00:00Z", "message_count":0
    }))
    .unwrap()
}
fn event() -> EventEnvelope {
    serde_json::from_value(serde_json::json!({
        "event_id":Uuid::new_v4().to_string(), "seq":1, "timestamp":"2026-10-03T00:00:01Z", "source":"Runtime",
        "kind":"turn.started", "thread_id":"thread", "turn_id":"turn",
        "event":{"TurnStarted":{"thread_id":"thread","turn_id":"turn","timestamp":"2026-10-03T00:00:01Z"}}
    })).unwrap()
}
fn item() -> ThreadItemEvent {
    serde_json::from_value(serde_json::json!({
        "seq":1,"eventId":Uuid::new_v4().to_string(),"threadId":"thread","turnId":"turn","timestamp":"2026-10-03T00:00:01Z",
        "event":{"type":"itemStarted","item":{"type":"userMessage","id":"item","text":"hello"}}
    })).unwrap()
}
fn checkpoint() -> ExtensionStateRecord {
    ExtensionStateRecord {
        extension_id: "test".into(),
        key: "checkpoint".into(),
        scope: ExtensionStoreScope::Thread {
            thread_id: "thread".into(),
        },
        schema_version: 1,
        value: serde_json::json!({"step":1}),
    }
}
async fn assert_writes_rejected(store: &MysqlSessionStore, artifact_id: &String) {
    let thread = "thread".to_string();
    let turn = "turn".to_string();
    assert!(store.create_thread(metadata()).await.is_err());
    assert!(store.update_thread_metadata(metadata()).await.is_err());
    assert!(store.archive_thread(&thread).await.is_err());
    assert!(store.append_event(&thread, &event()).await.is_err());
    assert!(store.append_item_event(&thread, &item()).await.is_err());
    assert!(
        store
            .append_extension_state(&thread, &checkpoint())
            .await
            .is_err()
    );
    let artifacts = store.context_artifact_store().unwrap();
    assert!(
        artifacts
            .create(CreateArtifactRequest {
                kind: ContextArtifactKind::ToolOutput,
                thread_id: &thread,
                turn_id: &turn,
                source_tool_id: None,
                label: None,
                bytes: b"rejected"
            })
            .is_err()
    );
    assert!(artifacts.append(&thread, artifact_id, b"rejected").is_err());
}

#[tokio::test]
#[ignore = "requires isolated MySQL"]
async fn every_durable_mutation_rejects_unfenced_and_superseded_handles() {
    let base = store().await;
    let thread = "thread".to_string();
    let turn = "turn".to_string();
    base.create_thread(metadata()).await.unwrap();
    let RuntimeOwnerClaim::Acquired(first) = base
        .claim_runtime_owner(
            Uuid::new_v4(),
            "127.0.0.1:4500".parse().unwrap(),
            Duration::from_secs(30),
        )
        .await
        .unwrap()
    else {
        panic!("owner")
    };
    let owned = base.with_runtime_owner(&first).unwrap();
    assert!(owned.for_tenant("different").is_err());
    let same_tenant = owned.for_tenant(base.tenant_id()).unwrap();
    use roder_api::extension::{ExtensionRegistryBuilder, RoderExtension};
    let mut registry = ExtensionRegistryBuilder::new();
    roder_ext_mysql_session::MysqlSessionExtension::from_store(owned.clone())
        .install(&mut registry)
        .unwrap();
    let runtime_store = registry.build().unwrap().thread_stores[0].create();
    runtime_store
        .update_thread_metadata(metadata())
        .await
        .unwrap();
    same_tenant
        .update_thread_metadata(metadata())
        .await
        .unwrap();
    owned.append_event(&thread, &event()).await.unwrap();
    owned.append_item_event(&thread, &item()).await.unwrap();
    owned
        .append_extension_state(&thread, &checkpoint())
        .await
        .unwrap();
    let artifacts = owned.context_artifact_store().unwrap();
    let artifact = artifacts
        .create(CreateArtifactRequest {
            kind: ContextArtifactKind::ToolOutput,
            thread_id: &thread,
            turn_id: &turn,
            source_tool_id: None,
            label: None,
            bytes: b"first",
        })
        .unwrap();
    artifacts.append(&thread, &artifact.id, b" second").unwrap();
    assert_writes_rejected(&base, &artifact.id).await;
    assert!(base.release_runtime_owner(&first).await.unwrap());
    let RuntimeOwnerClaim::Acquired(next) = base
        .claim_runtime_owner(
            Uuid::new_v4(),
            "127.0.0.1:4501".parse().unwrap(),
            Duration::from_secs(30),
        )
        .await
        .unwrap()
    else {
        panic!("owner")
    };
    assert_writes_rejected(&owned, &artifact.id).await;
    assert!(
        runtime_store
            .update_thread_metadata(metadata())
            .await
            .is_err()
    );
    assert_writes_rejected(&same_tenant, &artifact.id).await;
    let replacement = base.with_runtime_owner(&next).unwrap();
    replacement.append_event(&thread, &event()).await.unwrap();
    let snapshot = replacement.load_thread(&thread).await.unwrap().unwrap();
    assert_eq!(snapshot.events.len(), 2);
    assert_eq!(snapshot.item_events.len(), 1);
    assert_eq!(
        replacement
            .load_extension_states(&thread)
            .await
            .unwrap()
            .len(),
        1
    );
    let observed = artifacts
        .read_artifact(&thread, &artifact.id, 1, 10)
        .unwrap();
    assert!(observed.text.contains("first second"));
    assert!(!observed.text.contains("rejected"));
    assert!(replacement.archive_thread(&thread).await.unwrap());
}

#[tokio::test]
#[ignore = "requires isolated MySQL"]
async fn expiry_fences_writes_before_another_replica_claims() {
    let base = store().await;
    let RuntimeOwnerClaim::Acquired(lease) = base
        .claim_runtime_owner(
            Uuid::new_v4(),
            "127.0.0.1:4500".parse().unwrap(),
            Duration::from_secs(1),
        )
        .await
        .unwrap()
    else {
        panic!("owner")
    };
    let owned = base.with_runtime_owner(&lease).unwrap();
    owned.create_thread(metadata()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(owned.update_thread_metadata(metadata()).await.is_err());
    assert!(base.create_thread(metadata()).await.is_err());
}

#[tokio::test]
#[ignore = "requires isolated MySQL"]
async fn transcript_event_and_metadata_commit_together() {
    let base = store().await;
    let thread = "thread".to_string();
    let mut event = event();
    event.event = roder_api::events::RoderEvent::TranscriptItemAppended(
        roder_api::events::TranscriptItemAppended {
            thread_id: thread.clone(),
            turn_id: "turn".into(),
            timestamp: event.timestamp,
            item_type: "user_message".into(),
            item_index: Some(0),
            item: Some(roder_api::transcript::TranscriptItem::UserMessage(
                roder_api::transcript::UserMessage::text("hello"),
            )),
        },
    );
    event.kind = event.event.kind().to_string();
    assert!(base.append_event(&thread, &event).await.is_err());
    base.create_thread(metadata()).await.unwrap();
    assert!(
        base.load_thread(&thread)
            .await
            .unwrap()
            .unwrap()
            .events
            .is_empty()
    );
    base.append_event(&thread, &event).await.unwrap();
    let snapshot = base.load_thread(&thread).await.unwrap().unwrap();
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(snapshot.metadata.unwrap().message_count, 1);
}
