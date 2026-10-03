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
}
