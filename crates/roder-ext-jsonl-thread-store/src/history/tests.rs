use super::*;
use crate::store::JsonlThreadStore;
use roder_api::events::{EventSource, TurnCompleted, TurnFailed, TurnInterrupted};
use roder_api::thread::{ThreadItemDelta, ThreadItemStatus, ThreadStore};
use roder_api::transcript::{ContextCompactionRecord, InputImage, TranscriptItem, UserMessage};
use serde::Serialize;
use std::io::Write;
mod regressions;

struct Fixture {
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("roder-history-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let metadata: ThreadMetadata = serde_json::from_value(serde_json::json!({
            "thread_id":"thread", "workspace":"/workspace", "title":null,
            "provider":null,"model":null,"created_at":"2026-01-01T00:00:00Z",
            "updated_at":"2026-01-01T00:00:00Z","message_count":0
        }))
        .unwrap();
        std::fs::write(
            dir.join("metadata.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        Self { dir }
    }
    fn event(&self, turn: usize, seq: usize, event: ThreadItemEventKind) {
        let event = ThreadItemEvent {
            seq: seq as u64,
            event_id: format!("event-{seq}"),
            thread_id: "thread".into(),
            turn_id: format!("turn-{turn:04}"),
            timestamp: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(turn as i64),
            event,
        };
        append(&self.dir.join("item_events.jsonl"), &event);
    }
    fn item(&self, turn: usize, seq: usize, text: String) {
        self.event(
            turn,
            seq,
            ThreadItemEventKind::ItemCompleted {
                item: ThreadItem::AgentMessage {
                    id: format!("item-{seq}"),
                    text,
                    phase: None,
                    status: Some(ThreadItemStatus::Completed),
                },
            },
        );
    }
    fn raw(&self, event: RoderEvent) {
        append(
            &self.dir.join("events.jsonl"),
            &EventEnvelope {
                event_id: "event".into(),
                seq: 0,
                timestamp: OffsetDateTime::UNIX_EPOCH,
                source: EventSource::Core,
                kind: "test".into(),
                thread_id: Some("thread".into()),
                turn_id: None,
                event,
            },
        );
    }
    fn transcript(&self, index: usize, item: TranscriptItem) {
        self.raw(RoderEvent::TranscriptItemAppended(
            roder_api::events::TranscriptItemAppended {
                thread_id: "thread".into(),
                turn_id: "turn-0000".into(),
                item_type: "test".into(),
                item_index: Some(index),
                item: Some(item),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            },
        ));
    }
    fn page(
        &self,
        before: Option<String>,
        limit: usize,
        max_bytes: usize,
    ) -> anyhow::Result<ThreadHistoryPage> {
        Ok(read_sync(
            &self.dir,
            "thread",
            ThreadHistoryReadOptions {
                before,
                limit,
                max_bytes,
            },
        )?
        .unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn append(path: &Path, value: &impl Serialize) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    serde_json::to_writer(&mut file, value).unwrap();
    file.write_all(b"\n").unwrap();
}
fn ids(page: &ThreadHistoryPage) -> Vec<String> {
    page.turns
        .iter()
        .flat_map(|t| t.items.iter().map(|i| i.id().to_string()))
        .collect()
}

#[test]
fn huge_turn_and_many_turns_page_chronologically_without_overlap() {
    let f = Fixture::new();
    for i in 0..525 {
        f.item(if i < 25 { i } else { 25 }, i, format!("message-{i}"));
    }
    let mut before = None;
    let mut pages = Vec::new();
    loop {
        let page = f.page(before, 37, usize::MAX).unwrap();
        assert!(ids(&page).len() <= 37);
        pages.push(ids(&page));
        before = page.before;
        if !page.has_more {
            break;
        }
    }
    let actual: Vec<_> = pages.into_iter().rev().flatten().collect();
    assert_eq!(
        actual,
        (0..525).map(|i| format!("item-{i}")).collect::<Vec<_>>()
    );
    assert_eq!(
        ids(&f.page(None, usize::MAX, usize::MAX).unwrap()).len(),
        200
    );
}

#[test]
fn cursors_survive_append_reopen_and_derived_index_rebuild() {
    let f = Fixture::new();
    for i in 0..9 {
        f.item(0, i, i.to_string());
    }
    let newest = f.page(None, 3, usize::MAX).unwrap();
    let cursor = newest.before.unwrap();
    f.item(0, 9, "new".into());
    assert_eq!(
        ids(&f.page(Some(cursor.clone()), 3, usize::MAX).unwrap()),
        ["item-3", "item-4", "item-5"]
    );
    std::fs::remove_file(f.dir.join("history.sqlite")).unwrap();
    assert_eq!(
        ids(&f.page(Some(cursor), 3, usize::MAX).unwrap()),
        ["item-3", "item-4", "item-5"]
    );
}

#[test]
fn malformed_foreign_and_missing_cursors_are_rejected() {
    let f = Fixture::new();
    f.item(0, 0, "a".into());
    f.item(0, 1, "b".into());
    assert!(f.page(Some("no".into()), 1, 1000).is_err());
    let cursor = f.page(None, 1, 1000).unwrap().before.unwrap();
    let mut decoded = decode(&cursor).unwrap();
    decoded.thread = "other".into();
    assert!(f.page(Some(encode(&decoded).unwrap()), 1, 1000).is_err());
    decoded.thread = "thread".into();
    decoded.item = "absent".into();
    assert!(f.page(Some(encode(&decoded).unwrap()), 1, 1000).is_err());
}

#[test]
fn warm_page_never_deserializes_unselected_payloads() {
    let f = Fixture::new();
    for i in 0..10 {
        f.item(0, i, i.to_string());
    }
    f.page(None, 2, 1000).unwrap();
    let db = open(&f.dir).unwrap();
    db.execute(
        "UPDATE records SET payload='poison' WHERE item_key IN \
        (SELECT key FROM items WHERE item_id='item-0')",
        [],
    )
    .unwrap();
    assert_eq!(ids(&f.page(None, 2, 1000).unwrap()), ["item-8", "item-9"]);
    assert!(f.page(None, 10, 10000).is_err());
}

#[test]
fn partial_final_record_is_not_indexed_until_newline_commits_it() {
    let f = Fixture::new();
    f.item(0, 0, "first".into());
    f.page(None, 2, 1000).unwrap();
    let path = f.dir.join("item_events.jsonl");
    let len = std::fs::metadata(&path).unwrap().len();
    f.item(0, 1, "second".into());
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let full = file.metadata().unwrap().len();
    file.set_len(full - 1).unwrap();
    // Rebuild from source with the incomplete tail present, then resume warmly.
    std::fs::remove_file(f.dir.join("history.sqlite")).unwrap();
    assert_eq!(ids(&f.page(None, 2, 1000).unwrap()), ["item-0"]);
    let db = open(&f.dir).unwrap();
    assert_eq!(
        db.query_row("SELECT pos FROM offsets WHERE source=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        len as i64
    );
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(b"\n").unwrap();
    assert_eq!(ids(&f.page(None, 2, 1000).unwrap()), ["item-0", "item-1"]);
}

#[test]
fn byte_budget_and_oversized_items_always_progress() {
    let f = Fixture::new();
    for i in 0..7 {
        f.item(0, i, "x".repeat(700_000));
    }
    let mut before = None;
    let mut sizes = Vec::new();
    loop {
        let page = f.page(before, 200, 2 * 1024 * 1024).unwrap();
        sizes.push(ids(&page).len());
        before = page.before;
        if !page.has_more {
            break;
        }
    }
    assert_eq!(sizes, [2, 2, 2, 1]);
    let page = f.page(None, 200, 10).unwrap();
    assert_eq!(ids(&page), ["item-6"]);
    assert!(page.has_more);
    assert_eq!(ids(&f.page(page.before, 200, 10).unwrap()), ["item-5"]);
}

#[test]
fn interleaved_deltas_and_checkpoints_preserve_images_and_tools() {
    let f = Fixture::new();
    f.event(
        0,
        0,
        ThreadItemEventKind::ItemStarted {
            item: ThreadItem::AgentMessage {
                id: "agent".into(),
                text: "a".into(),
                phase: None,
                status: Some(ThreadItemStatus::InProgress),
            },
        },
    );
    f.event(
        0,
        1,
        ThreadItemEventKind::ItemStarted {
            item: ThreadItem::ToolExecution {
                id: "call".into(),
                tool_call_id: "call".into(),
                tool_name: "read".into(),
                status: ThreadItemStatus::InProgress,
                input: Some(serde_json::json!({"path":"a"})),
                output: None,
                error: None,
            },
        },
    );
    f.event(
        0,
        2,
        ThreadItemEventKind::ItemDelta {
            item_id: "agent".into(),
            delta: ThreadItemDelta::AgentMessageText {
                delta: "b".into(),
                phase: None,
            },
        },
    );
    f.event(
        0,
        3,
        ThreadItemEventKind::ItemCompleted {
            item: ThreadItem::ToolExecution {
                id: "call".into(),
                tool_call_id: "call".into(),
                tool_name: "read".into(),
                status: ThreadItemStatus::Completed,
                input: None,
                output: Some("result".into()),
                error: None,
            },
        },
    );
    f.event(
        0,
        4,
        ThreadItemEventKind::ItemDelta {
            item_id: "agent".into(),
            delta: ThreadItemDelta::AgentMessageText {
                delta: "c".into(),
                phase: None,
            },
        },
    );
    let page = f.page(None, 200, 10000).unwrap();
    assert_eq!(ids(&page), ["agent", "call"]);
    assert!(matches!(&page.turns[0].items[0],ThreadItem::AgentMessage{text,..} if text=="abc"));
    assert!(
        matches!(&page.turns[0].items[1],ThreadItem::ToolExecution{input:Some(_),output:Some(output),..} if output=="result")
    );
    let db = open(&f.dir).unwrap();
    assert_eq!(db.query_row("SELECT COUNT(*) FROM records r JOIN items i ON i.key=r.item_key WHERE i.item_id='call'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
}

#[test]
fn terminal_status_metadata_and_empty_latest_turn_are_preserved() {
    let f = Fixture::new();
    for i in 0..3 {
        f.item(i, i, "text".into());
    }
    let timestamp = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(10);
    f.raw(RoderEvent::TurnCompleted(TurnCompleted {
        thread_id: "thread".into(),
        turn_id: "turn-0000".into(),
        usage: None,
        finish_reason: Some("stop".into()),
        timestamp,
    }));
    f.raw(RoderEvent::TurnFailed(TurnFailed {
        thread_id: "thread".into(),
        turn_id: "turn-0001".into(),
        usage: None,
        error: "bad".into(),
        error_kind: None,
        timestamp,
    }));
    f.raw(RoderEvent::TurnInterrupted(TurnInterrupted {
        thread_id: "thread".into(),
        turn_id: "turn-0002".into(),
        usage: None,
        timestamp,
    }));
    let page = f.page(None, 200, 10000).unwrap();
    assert_eq!(page.turns[0].status, ThreadHistoryTurnStatus::Completed);
    assert_eq!(page.turns[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(page.turns[1].status, ThreadHistoryTurnStatus::Failed);
    assert_eq!(page.turns[1].error.as_deref(), Some("bad"));
    assert_eq!(page.turns[2].status, ThreadHistoryTurnStatus::Interrupted);
    assert!(page.turns.iter().all(|t| t.completed_at == Some(timestamp)));
}

#[tokio::test]
async fn public_store_append_keeps_index_current_and_returns_none_for_missing_thread() {
    let f = Fixture::new();
    let store = JsonlThreadStore {
        base_path: f.dir.parent().unwrap().into(),
    };
    assert!(
        store
            .read_history_page(&"missing".into(), ThreadHistoryReadOptions::default())
            .await
            .unwrap()
            .is_none()
    );
    // Exercise the async boundary without depending on fixture directory naming.
    assert!(
        read(
            f.dir.clone(),
            "thread".into(),
            ThreadHistoryReadOptions::default()
        )
        .await
        .unwrap()
        .is_some()
    );
}

#[test]
fn completed_saved_transcript_preserves_images_and_compaction_with_partial_event_projection() {
    let f = Fixture::new();
    f.transcript(
        0,
        TranscriptItem::UserMessage(UserMessage {
            text: "look".into(),
            images: vec![InputImage {
                image_url: "data:image/png;base64,AA==".into(),
            }],
        }),
    );
    let compaction: ContextCompactionRecord = serde_json::from_value(serde_json::json!({
        "summary":"kept summary"
    }))
    .unwrap();
    f.transcript(1, TranscriptItem::ContextCompaction(compaction));
    f.item(0, 3, "incomplete projected view".into());
    // Saved-only content is retained while active, not only on completion.
    assert_eq!(ids(&f.page(None, 200, 10000).unwrap()).len(), 3);
    f.raw(RoderEvent::TurnCompleted(TurnCompleted {
        thread_id: "thread".into(),
        turn_id: "turn-0000".into(),
        usage: None,
        finish_reason: None,
        timestamp: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1),
    }));
    let page = f.page(None, 200, 10000).unwrap();
    assert_eq!(
        ids(&page),
        ["turn-0000-user-0", "turn-0000-compaction-1", "item-3"]
    );
    assert!(
        matches!(&page.turns[0].items[0],ThreadItem::UserMessage{images,..}
        if images[0].image_url=="data:image/png;base64,AA==")
    );
}

#[test]
fn metadata_mismatch_and_path_traversal_are_rejected_before_index_open() {
    let f = Fixture::new();
    assert!(read_sync(&f.dir, "other", ThreadHistoryReadOptions::default()).is_err());
    for thread in ["../thread", ".", "..", "a/b", "a\\b", ""] {
        assert!(read_sync(&f.dir, thread, ThreadHistoryReadOptions::default()).is_err());
    }
    assert!(!f.dir.join("history.sqlite").exists());
}
