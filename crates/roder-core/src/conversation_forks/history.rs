use super::*;

/**
 * Selects the parent events that seed the child transcript: only
 * conversation-history records (turn lifecycle and transcript items), never
 * tool/approval/audit events, so nothing side-effectful is replayed. When
 * `from_turn_id` is set, events after that turn's records are dropped.
 */
pub(super) fn seed_events_for_child(
    events: &[EventEnvelope],
    from_turn_id: Option<&str>,
) -> anyhow::Result<Vec<EventEnvelope>> {
    let mut ordered: Vec<&EventEnvelope> = events.iter().collect();
    ordered.sort_by_key(|envelope| envelope.seq);

    let cutoff = match from_turn_id {
        Some(turn_id) => {
            let last = ordered
                .iter()
                .rposition(|envelope| envelope.turn_id.as_deref() == Some(turn_id))
                .ok_or_else(|| {
                    anyhow::anyhow!("turn {turn_id} was not found in the parent thread")
                })?;
            last + 1
        }
        None => ordered.len(),
    };

    Ok(ordered[..cutoff]
        .iter()
        .filter(|envelope| match &envelope.event {
            RoderEvent::TurnStarted(_)
            | RoderEvent::TurnCompleted(_)
            | RoderEvent::TurnFailed(_)
            | RoderEvent::TurnInterrupted(_) => true,
            RoderEvent::TranscriptItemAppended(event) => event
                .item
                .as_ref()
                .is_some_and(forkable_agent_transcript_item),
            _ => false,
        })
        .map(|envelope| (*envelope).clone())
        .collect())
}

fn forkable_agent_transcript_item(item: &roder_api::transcript::TranscriptItem) -> bool {
    match item {
        roder_api::transcript::TranscriptItem::UserMessage(_) => true,
        roder_api::transcript::TranscriptItem::AssistantMessage(message) => message
            .phase
            .as_deref()
            .is_none_or(|phase| phase.is_empty() || phase == "final_answer"),
        roder_api::transcript::TranscriptItem::ReasoningSummary(_)
        | roder_api::transcript::TranscriptItem::ToolCall(_)
        | roder_api::transcript::TranscriptItem::ToolResult(_)
        | roder_api::transcript::TranscriptItem::FileChange(_)
        | roder_api::transcript::TranscriptItem::ContextCompaction(_)
        | roder_api::transcript::TranscriptItem::Error(_)
        | roder_api::transcript::TranscriptItem::ProviderMetadata(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::{EventSource, TranscriptItemAppended, TurnCompleted, TurnStarted};
    use roder_api::transcript::{
        ContextCompactionRecord, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
    };

    fn envelope(seq: u64, turn_id: &str, event: RoderEvent) -> EventEnvelope {
        EventEnvelope {
            event_id: format!("event-{seq}"),
            seq,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: EventSource::Core,
            kind: event.kind().to_string(),
            thread_id: Some("parent".to_string()),
            turn_id: Some(turn_id.to_string()),
            event,
        }
    }

    fn turn_events(seq: u64, turn_id: &str, text: &str) -> Vec<EventEnvelope> {
        vec![
            envelope(
                seq,
                turn_id,
                RoderEvent::TurnStarted(TurnStarted {
                    thread_id: "parent".to_string(),
                    turn_id: turn_id.to_string(),
                    runtime_profile: Default::default(),
                    timestamp: OffsetDateTime::UNIX_EPOCH,
                }),
            ),
            envelope(
                seq + 1,
                turn_id,
                RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                    thread_id: "parent".to_string(),
                    turn_id: turn_id.to_string(),
                    item_type: "user_message".to_string(),
                    item_index: None,
                    item: Some(TranscriptItem::UserMessage(UserMessage::text(text))),
                    timestamp: OffsetDateTime::UNIX_EPOCH,
                }),
            ),
            envelope(
                seq + 2,
                turn_id,
                RoderEvent::TurnCompleted(TurnCompleted {
                    thread_id: "parent".to_string(),
                    turn_id: turn_id.to_string(),
                    usage: None,
                    finish_reason: Some("stop".to_string()),
                    timestamp: OffsetDateTime::UNIX_EPOCH,
                }),
            ),
        ]
    }

    #[test]
    fn seed_events_keep_conversation_records_only() {
        let mut events = turn_events(1, "turn-1", "hello");
        events.push(envelope(
            4,
            "turn-1",
            RoderEvent::ToolCallStarted(roder_api::events::ToolCallStarted {
                thread_id: "parent".to_string(),
                turn_id: "turn-1".to_string(),
                tool_id: "call-1".to_string(),
                tool_name: Some("shell".to_string()),
                display_payload: None,
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ));
        events.push(envelope(
            7,
            "turn-1",
            RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                thread_id: "parent".to_string(),
                turn_id: "turn-1".to_string(),
                item_type: "context_compaction".to_string(),
                item_index: None,
                item: Some(TranscriptItem::ContextCompaction(ContextCompactionRecord {
                    summary: "private parent compaction".to_string(),
                })),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ));
        events.push(envelope(
            5,
            "turn-1",
            RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                thread_id: "parent".to_string(),
                turn_id: "turn-1".to_string(),
                item_type: "tool_call".to_string(),
                item_index: None,
                item: Some(TranscriptItem::ToolCall(ToolCallRecord {
                    id: "spawn-call".to_string(),
                    name: "spawn_agent".to_string(),
                    arguments: "{}".to_string(),
                })),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ));
        events.push(envelope(
            6,
            "turn-1",
            RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                thread_id: "parent".to_string(),
                turn_id: "turn-1".to_string(),
                item_type: "tool_result".to_string(),
                item_index: None,
                item: Some(TranscriptItem::ToolResult(ToolResultRecord {
                    id: "spawn-call".to_string(),
                    name: Some("spawn_agent".to_string()),
                    result: "spawned".to_string(),
                    display_payload: None,
                    is_error: false,
                })),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ));

        let seeded = seed_events_for_child(&events, None).unwrap();

        assert_eq!(seeded.len(), 3, "tool records must not be replayed");
        assert!(
            seeded
                .iter()
                .all(|envelope| !matches!(envelope.event, RoderEvent::ToolCallStarted(_)))
        );
        assert!(seeded.iter().all(|envelope| {
            !matches!(
                &envelope.event,
                RoderEvent::TranscriptItemAppended(event)
                    if matches!(
                        event.item,
                        Some(
                            TranscriptItem::ToolCall(_)
                                | TranscriptItem::ToolResult(_)
                                | TranscriptItem::ContextCompaction(_)
                        )
                    )
            )
        }));
    }

    #[test]
    fn seed_events_truncate_at_requested_turn() {
        let mut events = turn_events(1, "turn-1", "first");
        events.extend(turn_events(10, "turn-2", "second"));

        let seeded = seed_events_for_child(&events, Some("turn-1")).unwrap();
        assert_eq!(seeded.len(), 3);
        assert!(
            seeded
                .iter()
                .all(|envelope| envelope.turn_id.as_deref() == Some("turn-1"))
        );

        let error = seed_events_for_child(&events, Some("missing-turn")).unwrap_err();
        assert!(error.to_string().contains("missing-turn"));
    }
}
