use roder_api::events::{EventEnvelope, EventSource, RoderEvent, TurnInterrupted};
use roder_api::inference::TokenUsage;
use roder_api::thread::project_turns_from_events;
use serde_json::json;
use time::OffsetDateTime;

fn interrupted(usage: TokenUsage) -> TurnInterrupted {
    serde_json::from_value(json!({
        "thread_id": "thread",
        "turn_id": "interrupted",
        "usage": usage,
        "timestamp": "1970-01-01T00:00:00Z"
    }))
    .unwrap()
}

#[test]
fn interrupted_usage_survives_event_serialization_and_turn_projection() {
    let usage = TokenUsage {
        prompt_tokens: 100,
        completion_tokens: 20,
        total_tokens: 120,
        cached_prompt_tokens: 30,
        cache_creation_prompt_tokens: 10,
        service_tier: Some("priority".into()),
        ..TokenUsage::default()
    };
    let event = RoderEvent::TurnInterrupted(interrupted(usage.clone()));
    let persisted = serde_json::to_vec(&event).unwrap();
    let event: RoderEvent = serde_json::from_slice(&persisted).unwrap();
    let envelope = EventEnvelope {
        event_id: "interruption".into(),
        seq: 1,
        timestamp: OffsetDateTime::UNIX_EPOCH,
        source: EventSource::Core,
        kind: event.kind().into(),
        thread_id: event.thread_id().cloned(),
        turn_id: event.turn_id().cloned(),
        event,
    };

    let turns = project_turns_from_events(&"thread".into(), &[envelope]);

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].turn_id, "interrupted");
    assert_eq!(turns[0].completed_at, Some(OffsetDateTime::UNIX_EPOCH));
    assert_eq!(turns[0].usage, Some(usage));
}

#[test]
fn interruption_without_reported_usage_remains_unknown() {
    let event: TurnInterrupted = serde_json::from_value(json!({
        "thread_id": "thread",
        "turn_id": "interrupted",
        "timestamp": "1970-01-01T00:00:00Z"
    }))
    .unwrap();
    assert_eq!(event.usage, None);
    let serialized = serde_json::to_value(event).unwrap();

    assert!(serialized.get("usage").is_none());
}
