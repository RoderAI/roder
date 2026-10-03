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

    // Events + item events round-trip (idempotent on event identity).
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

#[tokio::test(flavor = "multi_thread")]
async fn runtime_sequence_restart_preserves_distinct_events_and_terminal_history() {
    let Some(url) = test_url() else {
        eprintln!("skipping: RODER_MYSQL_TEST_URL not set");
        return;
    };
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let config = MysqlSessionConfig {
        database_url: url,
        tenant_id: format!("restart-{suffix}"),
        max_connections: Some(2),
    };
    let thread = format!("thread-{suffix}");
    let store = MysqlSessionStore::connect(&config).await.expect("connect");
    store
        .create_thread(metadata(&thread, "2026-06-12T00:00:00Z"))
        .await
        .expect("create");
    let mut terminal = envelope(&thread, 1);
    terminal.event_id = "terminal-before-restart".into();
    terminal.event =
        roder_api::events::RoderEvent::TurnInterrupted(roder_api::events::TurnInterrupted {
            thread_id: thread.clone(),
            turn_id: "turn-1".into(),
            timestamp: terminal.timestamp,
        });
    terminal.kind = terminal.event.kind().to_string();
    store
        .append_event(&thread, &terminal)
        .await
        .expect("terminal event");
    drop(store);
    let restarted = MysqlSessionStore::connect(&config)
        .await
        .expect("reconnect");
    let mut next = envelope(&thread, 1);
    next.event_id = "distinct-after-restart".into();
    restarted
        .append_event(&thread, &next)
        .await
        .expect("new event with reused runtime sequence");
    let mut replay = next.clone();
    replay.event = terminal.event.clone();
    replay.kind = replay.event.kind().to_string();
    restarted
        .append_event(&thread, &replay)
        .await
        .expect("idempotent replay");
    let snapshot = restarted
        .load_thread(&thread)
        .await
        .expect("load")
        .expect("exists");
    assert_eq!(
        snapshot.events.len(),
        2,
        "distinct event identities must survive a runtime counter reset"
    );
    assert!(
        snapshot
            .events
            .iter()
            .any(|event| event.event_id == terminal.event_id
                && matches!(
                    event.event,
                    roder_api::events::RoderEvent::TurnInterrupted(_)
                ))
    );
    assert!(
        snapshot
            .events
            .iter()
            .any(|event| event.event_id == next.event_id
                && matches!(event.event, roder_api::events::RoderEvent::TurnStarted(_)))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn independent_writers_preserve_distinct_event_and_item_identities() {
    let Some(url) = test_url() else { return };
    let config = MysqlSessionConfig {
        database_url: url,
        tenant_id: format!("writers-{}", uuid::Uuid::new_v4()),
        max_connections: Some(2),
    };
    let left = MysqlSessionStore::connect(&config)
        .await
        .expect("left store");
    let right = MysqlSessionStore::connect(&config)
        .await
        .expect("right store");
    let thread = "concurrent-thread".to_string();
    left.create_thread(metadata(&thread, "2026-06-12T00:00:00Z"))
        .await
        .expect("create");
    let mut a = envelope(&thread, 1);
    a.event_id = "left-event".repeat(100);
    let mut b = envelope(&thread, 1);
    b.event_id = "right-event".into();
    let (a_result, b_result) = tokio::join!(
        left.append_event(&thread, &a),
        right.append_event(&thread, &b)
    );
    a_result.expect("left event");
    b_result.expect("right event");
    let mut ia = item_event(&thread, 1);
    ia.event_id = "left-item-event".repeat(100);
    let mut ib = item_event(&thread, 1);
    ib.event_id = "right-item-event".into();
    let (a_result, b_result) = tokio::join!(
        left.append_item_event(&thread, &ia),
        right.append_item_event(&thread, &ib)
    );
    a_result.expect("left item");
    b_result.expect("right item");
    right
        .append_item_event(&thread, &ia)
        .await
        .expect("duplicate item");
    right
        .append_event(&thread, &a)
        .await
        .expect("duplicate event");
    let snapshot = left
        .load_thread(&thread)
        .await
        .expect("load")
        .expect("exists");
    assert_eq!(snapshot.events.len(), 2);
    assert_eq!(snapshot.item_events.len(), 2);
    assert!(snapshot.events[0].seq < snapshot.events[1].seq);
    assert!(snapshot.item_events[0].seq < snapshot.item_events[1].seq);
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_transcript_event_does_not_inflate_message_count() {
    let Some(url) = test_url() else { return };
    let config = MysqlSessionConfig {
        database_url: url,
        tenant_id: format!("replay-{}", uuid::Uuid::new_v4()),
        max_connections: Some(2),
    };
    let store = MysqlSessionStore::connect(&config).await.expect("store");
    let thread = "replayed-transcript".to_string();
    store
        .create_thread(metadata(&thread, "2026-06-12T00:00:00Z"))
        .await
        .expect("create");
    let mut event = envelope(&thread, 1);
    event.event = roder_api::events::RoderEvent::TranscriptItemAppended(
        roder_api::events::TranscriptItemAppended {
            thread_id: thread.clone(),
            turn_id: "turn-1".into(),
            timestamp: event.timestamp,
            item_type: "user_message".into(),
            item_index: Some(0),
            item: Some(roder_api::transcript::TranscriptItem::UserMessage(
                roder_api::transcript::UserMessage::text("Hello"),
            )),
        },
    );
    event.kind = event.event.kind().to_string();
    store.append_event(&thread, &event).await.expect("append");
    store.append_event(&thread, &event).await.expect("replay");
    let snapshot = store
        .load_thread(&thread)
        .await
        .expect("load")
        .expect("exists");
    assert_eq!(snapshot.metadata.expect("metadata").message_count, 1);
    assert_eq!(snapshot.events.len(), 1);
}
