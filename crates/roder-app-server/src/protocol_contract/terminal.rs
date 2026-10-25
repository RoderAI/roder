use std::collections::HashMap;

use roder_api::events::{EventEnvelope, RoderEvent};
use roder_protocol::Turn;

/// A completion timestamp records termination, not necessarily success.
pub(super) fn restore_outcomes(turns: &mut [Turn], events: &[EventEnvelope]) {
    let mut outcomes = HashMap::new();
    for envelope in events {
        let id = match &envelope.event {
            RoderEvent::TurnCompleted(event) => &event.turn_id,
            RoderEvent::TurnFailed(event) => &event.turn_id,
            RoderEvent::TurnInterrupted(event) => &event.turn_id,
            _ => continue,
        };
        outcomes.insert(id.as_str(), &envelope.event);
    }
    for turn in turns {
        match outcomes.get(turn.id.as_str()) {
            Some(RoderEvent::TurnInterrupted(event)) => {
                turn.status = "interrupted".into();
                turn.duration_ms = None;
                turn.error = None;
                turn.completed_at = Some(event.timestamp.unix_timestamp());
                turn.usage = event.usage.clone();
            }
            Some(RoderEvent::TurnFailed(event)) => {
                turn.status = "failed".into();
                turn.duration_ms = None;
                turn.error = Some(serde_json::json!({ "message": event.error }));
                turn.completed_at = Some(event.timestamp.unix_timestamp());
            }
            Some(RoderEvent::TurnCompleted(event)) => {
                turn.status = "completed".into();
                turn.error = None;
                turn.completed_at = Some(event.timestamp.unix_timestamp());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::{EventSource, TurnFailed, TurnInterrupted};
    use roder_api::thread::{ThreadSnapshot, project_turns_from_events};
    use time::OffsetDateTime;

    #[test]
    fn restored_terminal_outcomes_match_live_notifications() {
        let timestamp = OffsetDateTime::UNIX_EPOCH;
        let events = vec![
            RoderEvent::TurnInterrupted(TurnInterrupted {
                thread_id: "thread".into(),
                turn_id: "stopped".into(),
                usage: None,
                timestamp,
            }),
            RoderEvent::TurnFailed(TurnFailed {
                thread_id: "thread".into(),
                turn_id: "failed".into(),
                timestamp,
                error: "Provider unavailable".into(),
                error_kind: None,
                usage: None,
            }),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, event)| EventEnvelope {
            event_id: format!("event-{index}"),
            seq: index as u64,
            timestamp,
            source: EventSource::Runtime,
            kind: String::new(),
            thread_id: Some("thread".into()),
            turn_id: None,
            event,
        })
        .collect::<Vec<_>>();
        let records = project_turns_from_events(&"thread".into(), &events);
        assert!(records.iter().all(|record| record.completed_at.is_some()));
        let turns = super::super::protocol_turns_from_snapshot(&ThreadSnapshot {
            metadata: None,
            turns: records,
            events,
            item_events: Vec::new(),
            extension_states: Vec::new(),
        });
        assert_eq!(turns[0].status, "interrupted");
        assert_eq!(turns[0].error, None);
        assert_eq!(turns[0].duration_ms, None);
        assert_eq!(turns[1].duration_ms, None);
        assert_eq!(turns[1].status, "failed");
        assert_eq!(
            turns[1].error,
            Some(serde_json::json!({"message": "Provider unavailable"}))
        );
    }

    #[test]
    fn restoring_interrupted_outcome_restores_usage_from_persisted_event() {
        let usage = roder_api::inference::TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            cached_prompt_tokens: 30,
            cache_creation_prompt_tokens: 10,
            ..Default::default()
        };
        let interruption: TurnInterrupted = serde_json::from_value(serde_json::json!({
            "thread_id": "thread",
            "turn_id": "stopped",
            "usage": usage,
            "timestamp": "1970-01-01T00:00:01Z"
        }))
        .unwrap();
        let event = RoderEvent::TurnInterrupted(interruption);
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
        let mut turns = vec![Turn {
            id: "stopped".into(),
            items: vec![],
            items_view: "full".into(),
            status: "completed".into(),
            error: None,
            started_at: Some(0),
            completed_at: Some(1),
            duration_ms: Some(1000),
            usage: None,
            finish_reason: None,
        }];

        restore_outcomes(&mut turns, &[envelope]);

        assert_eq!(turns[0].status, "interrupted");
        assert_eq!(turns[0].usage, Some(usage));
    }
}
