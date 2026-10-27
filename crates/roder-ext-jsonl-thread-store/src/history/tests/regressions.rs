use super::*;
use roder_api::transcript::{AssistantMessage, ToolCallRecord, ToolResultRecord};

fn message(text: &str) -> ThreadItem {
    ThreadItem::AgentMessage {
        id: "shared".into(),
        text: text.into(),
        phase: None,
        status: Some(ThreadItemStatus::InProgress),
    }
}

#[test]
fn ordinary_completion_retains_live_cursor_and_projected_identity() {
    let f = Fixture::new();
    for i in 0..8 {
        f.item(0, i, format!("text-{i}"));
    }
    let page = f.page(None, 3, 10000).unwrap();
    assert_eq!(ids(&page), ["item-5", "item-6", "item-7"]);
    let before = page.before;
    for i in 0..8 {
        f.transcript(
            i,
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: format!("text-{i}"),
                phase: None,
            }),
        );
    }
    f.raw(RoderEvent::TurnCompleted(TurnCompleted {
        thread_id: "thread".into(),
        turn_id: "turn-0000".into(),
        usage: None,
        finish_reason: Some("stop".into()),
        timestamp: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1),
    }));
    let older = f.page(before, 3, 10000).unwrap();
    assert_eq!(ids(&older), ["item-2", "item-3", "item-4"]);
    assert_eq!(older.turns[0].status, ThreadHistoryTurnStatus::Completed);
    assert_eq!(
        ids(&f.page(None, 200, 10000).unwrap()),
        (0..8).map(|i| format!("item-{i}")).collect::<Vec<_>>()
    );
}

#[test]
fn reused_started_id_creates_new_generation_and_routes_deltas_to_latest() {
    let f = Fixture::new();
    f.event(
        0,
        0,
        ThreadItemEventKind::ItemStarted {
            item: message("first"),
        },
    );
    f.event(
        0,
        1,
        ThreadItemEventKind::ItemCompleted {
            item: message("first done"),
        },
    );
    f.event(
        0,
        2,
        ThreadItemEventKind::ItemStarted {
            item: message("second"),
        },
    );
    f.event(
        0,
        3,
        ThreadItemEventKind::ItemDelta {
            item_id: "shared".into(),
            delta: ThreadItemDelta::AgentMessageText {
                delta: " delta".into(),
                phase: None,
            },
        },
    );
    f.event(
        0,
        4,
        ThreadItemEventKind::ItemCompleted {
            item: message("second done"),
        },
    );
    let latest = f.page(None, 1, 10000).unwrap();
    assert_eq!(ids(&latest), ["shared::duplicate-2"]);
    assert!(
        matches!(&latest.turns[0].items[0],ThreadItem::AgentMessage{text,..} if text=="second done")
    );
    let older = f.page(latest.before, 1, 10000).unwrap();
    assert_eq!(ids(&older), ["shared"]);
    assert!(
        matches!(&older.turns[0].items[0],ThreadItem::AgentMessage{text,..} if text=="first done")
    );
    std::fs::remove_file(f.dir.join("history.sqlite")).unwrap();
    assert_eq!(
        ids(&f.page(None, 200, 10000).unwrap()),
        ["shared", "shared::duplicate-2"]
    );
}

#[test]
fn saved_tool_call_and_result_are_one_item_preserving_input_and_name() {
    let f = Fixture::new();
    f.transcript(
        0,
        TranscriptItem::ToolCall(ToolCallRecord {
            id: "call".into(),
            name: "read_file".into(),
            arguments: r#"{"path":"a"}"#.into(),
        }),
    );
    f.transcript(
        1,
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "call".into(),
            name: None,
            result: "content".into(),
            is_error: false,
            display_payload: None,
        }),
    );
    let page = f.page(None, 200, 10000).unwrap();
    assert_eq!(ids(&page), ["call"]);
    assert!(matches!(&page.turns[0].items[0],ThreadItem::ToolExecution {
        tool_call_id,tool_name,input:Some(input),output:Some(output),..
    } if tool_call_id=="call" && tool_name=="read_file" && input["path"]=="a" && output=="content"));
}

#[test]
fn warm_reads_seek_past_old_source_payloads() {
    let f = Fixture::new();
    for i in 0..5 {
        f.item(0, i, format!("text-{i}"));
    }
    f.page(None, 2, 10000).unwrap();
    let mut source = std::fs::OpenOptions::new()
        .write(true)
        .open(f.dir.join("item_events.jsonl"))
        .unwrap();
    // The already indexed prefix is deliberately no longer valid JSON.
    source.write_all(b"poison").unwrap();
    assert_eq!(ids(&f.page(None, 2, 10000).unwrap()), ["item-3", "item-4"]);
    f.item(0, 5, "new".into());
    assert_eq!(ids(&f.page(None, 2, 10000).unwrap()), ["item-4", "item-5"]);
}

#[tokio::test]
async fn public_append_indexes_before_returning_and_concurrent_pages_are_safe() {
    let f = Fixture::new();
    let store = std::sync::Arc::new(JsonlThreadStore {
        base_path: f.dir.join("store"),
    });
    let metadata: ThreadMetadata =
        serde_json::from_slice(&std::fs::read(f.dir.join("metadata.json")).unwrap()).unwrap();
    store.create_thread(metadata).await.unwrap();
    let thread = "thread".to_string();
    for seq in 0..12 {
        store
            .append_item_event(
                &thread,
                &ThreadItemEvent {
                    seq,
                    event_id: format!("e-{seq}"),
                    thread_id: thread.clone(),
                    turn_id: "turn".into(),
                    timestamp: OffsetDateTime::UNIX_EPOCH,
                    event: ThreadItemEventKind::ItemCompleted {
                        item: ThreadItem::AgentMessage {
                            id: format!("item-{seq}"),
                            text: "text".into(),
                            phase: None,
                            status: None,
                        },
                    },
                },
            )
            .await
            .unwrap();
    }
    let dir = store.base_path.join("thread");
    let db = open(&dir).unwrap();
    let offset: i64 = db
        .query_row("SELECT pos FROM offsets WHERE source=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        offset as u64,
        std::fs::metadata(dir.join("item_events.jsonl"))
            .unwrap()
            .len()
    );
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let store = store.clone();
        let thread = thread.clone();
        tasks.push(tokio::spawn(async move {
            store
                .read_history_page(
                    &thread,
                    ThreadHistoryReadOptions {
                        limit: 3,
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(ids(&task.await.unwrap()), ["item-9", "item-10", "item-11"]);
    }
}

#[test]
fn byte_excluded_payload_is_not_deserialized() {
    let f = Fixture::new();
    f.item(0, 0, "old".repeat(1000));
    f.item(0, 1, "new".repeat(1000));
    f.page(None, 2, 10000).unwrap();
    let db = open(&f.dir).unwrap();
    db.execute(
        "UPDATE records SET payload='poison' WHERE item_key IN \
        (SELECT key FROM items WHERE item_id='item-0')",
        [],
    )
    .unwrap();
    let page = f.page(None, 200, 4000).unwrap();
    assert_eq!(ids(&page), ["item-1"]);
    assert!(page.has_more);
}

#[test]
fn reused_completed_id_without_start_preserves_distinct_messages() {
    let f = Fixture::new();
    f.event(
        0,
        0,
        ThreadItemEventKind::ItemCompleted {
            item: message("first"),
        },
    );
    f.event(
        0,
        1,
        ThreadItemEventKind::ItemCompleted {
            item: message("second"),
        },
    );
    assert_eq!(
        ids(&f.page(None, 200, 10000).unwrap()),
        ["shared", "shared::duplicate-2"]
    );
}

#[test]
fn repeated_saved_messages_are_not_all_suppressed_by_one_projected_equivalent() {
    let f = Fixture::new();
    for i in 0..2 {
        f.transcript(
            i,
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "same".into(),
                phase: None,
            }),
        );
    }
    f.item(0, 0, "same".into());
    assert_eq!(ids(&f.page(None, 200, 10000).unwrap()).len(), 2);
}

#[test]
fn latest_empty_turn_is_included_on_initial_page_only() {
    let f = Fixture::new();
    f.item(0, 0, "old".into());
    f.raw(RoderEvent::TurnStarted(roder_api::events::TurnStarted {
        thread_id: "thread".into(),
        turn_id: "turn-new".into(),
        runtime_profile: Default::default(),
        timestamp: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(20),
    }));
    let page = f.page(None, 200, 10000).unwrap();
    assert_eq!(page.turns.len(), 2);
    assert_eq!(page.turns[1].turn_id, "turn-new");
    assert!(page.turns[1].items.is_empty());
    assert_eq!(page.turns[1].status, ThreadHistoryTurnStatus::InProgress);
}
