use roder_api::events::{
    EventEnvelope, EventSource, RoderEvent, ThreadId, TranscriptItemAppended, TurnCompleted,
    TurnInterrupted, TurnStarted,
};
use roder_api::inference::{RuntimeProfile, TokenUsage};
use roder_api::thread::project_turns_from_events;
use roder_api::transcript::{AssistantMessage, TranscriptItem, UserMessage};
use time::OffsetDateTime;

fn envelope(seq: u64, timestamp: OffsetDateTime, event: RoderEvent) -> EventEnvelope {
    let kind = event.kind().to_string();
    EventEnvelope {
        event_id: format!("event-{seq}"),
        seq,
        timestamp,
        source: EventSource::Core,
        kind,
        thread_id: event.thread_id().cloned(),
        turn_id: event.turn_id().cloned(),
        event,
    }
}

#[test]
fn extension_authors_can_project_turns_from_raw_events() {
    let thread_id: ThreadId = "thread-a".to_string();
    let first_turn = "turn-a".to_string();
    let second_turn = "turn-b".to_string();
    let started_at = OffsetDateTime::UNIX_EPOCH;
    let appended_at = started_at + time::Duration::seconds(1);
    let completed_at = started_at + time::Duration::seconds(2);
    let interrupted_at = started_at + time::Duration::seconds(3);
    let usage = TokenUsage::new(10, 5, 15);

    let events = vec![
        envelope(
            1,
            started_at,
            RoderEvent::TurnStarted(TurnStarted {
                thread_id: thread_id.clone(),
                turn_id: first_turn.clone(),
                runtime_profile: RuntimeProfile::Interactive,
                timestamp: started_at,
            }),
        ),
        envelope(
            2,
            appended_at,
            RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                thread_id: thread_id.clone(),
                turn_id: first_turn.clone(),
                item_type: "user_message".to_string(),
                item_index: Some(0),
                item: Some(TranscriptItem::UserMessage(UserMessage::text("hello"))),
                timestamp: appended_at,
            }),
        ),
        envelope(
            3,
            completed_at,
            RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                thread_id: thread_id.clone(),
                turn_id: first_turn.clone(),
                item_type: "assistant_message".to_string(),
                item_index: Some(1),
                item: Some(TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "hi".to_string(),
                    phase: None,
                })),
                timestamp: completed_at,
            }),
        ),
        envelope(
            4,
            completed_at,
            RoderEvent::TurnCompleted(TurnCompleted {
                thread_id: thread_id.clone(),
                turn_id: first_turn.clone(),
                usage: Some(usage.clone()),
                timestamp: completed_at,
            }),
        ),
        envelope(
            5,
            interrupted_at,
            RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                thread_id: thread_id.clone(),
                turn_id: second_turn.clone(),
                item_type: "assistant_message".to_string(),
                item_index: None,
                item: None,
                timestamp: interrupted_at,
            }),
        ),
        envelope(
            6,
            interrupted_at,
            RoderEvent::TurnInterrupted(TurnInterrupted {
                thread_id: thread_id.clone(),
                turn_id: second_turn.clone(),
                timestamp: interrupted_at,
            }),
        ),
    ];

    let turns = project_turns_from_events(&thread_id, &events);

    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].thread_id, thread_id);
    assert_eq!(turns[0].turn_id, first_turn);
    assert_eq!(turns[0].created_at, started_at);
    assert_eq!(turns[0].completed_at, Some(completed_at));
    assert_eq!(turns[0].usage, Some(usage));
    assert_eq!(
        turns[0].items,
        vec![
            TranscriptItem::UserMessage(UserMessage::text("hello")),
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "hi".to_string(),
                phase: None,
            }),
        ]
    );
    assert_eq!(turns[1].turn_id, second_turn);
    assert_eq!(turns[1].created_at, interrupted_at);
    assert_eq!(turns[1].completed_at, Some(interrupted_at));
    assert!(turns[1].items.is_empty());
}
