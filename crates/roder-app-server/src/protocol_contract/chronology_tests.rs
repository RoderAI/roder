use super::protocol_turns_from_snapshot;
use roder_api::thread::{
    ThreadItemDelta, ThreadItemEvent, ThreadItemEventKind, ThreadSnapshot, TurnRecord,
};
use roder_api::transcript::{TranscriptItem, UserMessage};
use time::{Duration, OffsetDateTime};

#[test]
fn restored_event_only_turns_interleave_with_saved_turns_at_subsecond_precision() {
    let timestamp = OffsetDateTime::UNIX_EPOCH;
    let record = |id: &str, millis: i64| TurnRecord {
        thread_id: "thread".into(),
        turn_id: id.into(),
        items: vec![TranscriptItem::UserMessage(UserMessage::text(id))],
        created_at: timestamp + Duration::milliseconds(millis),
        completed_at: Some(timestamp + Duration::milliseconds(millis + 1)),
        usage: None,
        finish_reason: Some("stop".into()),
    };
    let item_event = |id: &str, seq: u64, millis: i64| ThreadItemEvent {
        seq,
        event_id: format!("event-{seq}"),
        thread_id: "thread".into(),
        turn_id: id.into(),
        timestamp: timestamp + Duration::milliseconds(millis),
        event: ThreadItemEventKind::ItemDelta {
            item_id: format!("{id}-message"),
            delta: ThreadItemDelta::AgentMessageText {
                delta: id.into(),
                phase: Some("final".into()),
            },
        },
    };
    let turns = protocol_turns_from_snapshot(&ThreadSnapshot {
        metadata: None,
        events: Vec::new(),
        turns: vec![record("latest", 900), record("middle", 500)],
        item_events: vec![
            item_event("earliest", 1, 100),
            item_event("between", 2, 700),
            item_event("latest", 3, 900),
        ],
        extension_states: Vec::new(),
    });
    assert_eq!(
        turns
            .iter()
            .map(|turn| turn.id.as_str())
            .collect::<Vec<_>>(),
        ["earliest", "middle", "between", "latest"]
    );
    assert_eq!(
        turns[3].items.len(),
        1,
        "saved turns must not be duplicated by item events"
    );
}
