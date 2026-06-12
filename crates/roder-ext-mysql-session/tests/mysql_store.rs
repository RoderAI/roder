//! Live MySQL integration test. Runs only when `RODER_MYSQL_TEST_URL` is
//! set, e.g.:
//!
//! ```sh
//! docker run -d --rm --name roder-mysql-test -e MYSQL_ROOT_PASSWORD=root \
//!   -e MYSQL_DATABASE=roder_test -p 13306:3306 mysql:8
//! RODER_MYSQL_TEST_URL=mysql://root:root@127.0.0.1:13306/roder_test \
//!   cargo test -p roder-ext-mysql-session --test mysql_store
//! ```

use roder_api::events::EventEnvelope;
use roder_api::thread::{ThreadItemEvent, ThreadListOptions, ThreadMetadata, ThreadStore};
use roder_ext_mysql_session::{MysqlSessionConfig, MysqlSessionStore};

fn test_url() -> Option<String> {
    std::env::var("RODER_MYSQL_TEST_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
}

fn metadata(thread_id: &str, updated_at: &str) -> ThreadMetadata {
    serde_json::from_value(serde_json::json!({
        "thread_id": thread_id,
        "title": null,
        "workspace": "/tmp/roder-mysql-test",
        "provider": "codex",
        "model": "gpt-5.5",
        "created_at": "2026-06-12T00:00:00Z",
        "updated_at": updated_at,
        "message_count": 0
    }))
    .expect("build thread metadata")
}

fn envelope(thread_id: &str, seq: u64) -> EventEnvelope {
    serde_json::from_value(serde_json::json!({
        "event_id": format!("event-{seq}"),
        "seq": seq,
        "timestamp": "2026-06-12T00:00:01Z",
        "source": "Runtime",
        "kind": "turn.started",
        "thread_id": thread_id,
        "turn_id": "turn-1",
        "event": {
            "TurnStarted": {
                "thread_id": thread_id,
                "turn_id": "turn-1",
                "timestamp": "2026-06-12T00:00:01Z"
            }
        }
    }))
    .expect("build event envelope")
}

fn item_event(thread_id: &str, seq: u64) -> ThreadItemEvent {
    serde_json::from_value(serde_json::json!({
        "seq": seq,
        "eventId": format!("item-event-{seq}"),
        "threadId": thread_id,
        "turnId": "turn-1",
        "timestamp": "2026-06-12T00:00:02Z",
        "event": {
            "type": "itemStarted",
            "item": { "type": "userMessage", "id": format!("item-{seq}"), "text": "hello" }
        }
    }))
    .expect("build item event")
}

#[tokio::test(flavor = "multi_thread")]
async fn round_trips_threads_events_and_archive() {
    let Some(url) = test_url() else {
        eprintln!("skipping: RODER_MYSQL_TEST_URL not set");
        return;
    };

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let tenant = format!("tenant-{suffix}");
    let config = MysqlSessionConfig {
        database_url: url,
        tenant_id: tenant.clone(),
        max_connections: Some(2),
    };
    let store = MysqlSessionStore::connect(&config).await.expect("connect");

    let thread_a = format!("thread-a-{suffix}");
    let thread_b = format!("thread-b-{suffix}");
    store
        .create_thread(metadata(&thread_a, "2026-06-12T00:00:00Z"))
        .await
        .expect("create thread a");
    store
        .create_thread(metadata(&thread_b, "2026-06-12T01:00:00Z"))
        .await
        .expect("create thread b");

    // Newest updated_at first.
    let listed = store.list_threads().await.expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].thread_id, thread_b);

    // Pagination.
    let page = store
        .list_threads_page(ThreadListOptions {
            limit: Some(1),
            ..Default::default()
        })
        .await
        .expect("page");
    assert_eq!(page.threads.len(), 1);
    assert!(page.next_cursor.is_some());

    // Events + item events round-trip (idempotent on seq).
    store
        .append_event(&thread_a, &envelope(&thread_a, 1))
        .await
        .expect("append event");
    store
        .append_event(&thread_a, &envelope(&thread_a, 1))
        .await
        .expect("append event again");
    store
        .append_item_event(&thread_a, &item_event(&thread_a, 1))
        .await
        .expect("append item event");

    let snapshot = store
        .load_thread(&thread_a)
        .await
        .expect("load thread")
        .expect("snapshot present");
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(snapshot.item_events.len(), 1);
    assert_eq!(
        snapshot.metadata.as_ref().map(|m| m.thread_id.clone()),
        Some(thread_a.clone())
    );

    // Tenant isolation: another tenant sees nothing.
    let other = store
        .for_tenant(&format!("other-{suffix}"))
        .expect("tenant");
    assert!(other.list_threads().await.expect("other list").is_empty());

    // Archive hides the thread.
    assert!(store.archive_thread(&thread_a).await.expect("archive"));
    assert!(
        store
            .load_thread(&thread_a)
            .await
            .expect("load archived")
            .is_none()
    );
    let listed = store.list_threads().await.expect("list after archive");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].thread_id, thread_b);
}
