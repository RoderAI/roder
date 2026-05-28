use time::OffsetDateTime;

use crate::events::{EventEnvelope, RoderEvent, ThreadId};

use super::{
    ThreadItem, ThreadItemDelta, ThreadItemEvent, ThreadItemEventKind, ThreadItemStatus,
    ThreadItemTurnRecord, TurnRecord,
};

/// Projects raw runtime event envelopes into per-turn transcript records.
///
/// Thread stores that persist `EventEnvelope` logs can use this to populate a
/// snapshot's `turns` field without duplicating the runtime replay rules.
pub fn project_turns_from_events(
    thread_id: &ThreadId,
    events: &[EventEnvelope],
) -> Vec<TurnRecord> {
    let mut turns = Vec::new();
    for envelope in events {
        match &envelope.event {
            RoderEvent::TurnStarted(event) => {
                ensure_turn_record(&mut turns, thread_id, &event.turn_id, event.timestamp);
            }
            RoderEvent::TranscriptItemAppended(event) => {
                let turn =
                    ensure_turn_record(&mut turns, thread_id, &event.turn_id, event.timestamp);
                if let Some(item) = &event.item {
                    turn.items.push(item.clone());
                }
            }
            RoderEvent::TurnCompleted(event) => {
                let turn =
                    ensure_turn_record(&mut turns, thread_id, &event.turn_id, event.timestamp);
                turn.completed_at = Some(event.timestamp);
                turn.usage = event.usage.clone();
            }
            RoderEvent::TurnFailed(event) => {
                let turn =
                    ensure_turn_record(&mut turns, thread_id, &event.turn_id, event.timestamp);
                turn.completed_at = Some(event.timestamp);
                turn.usage = event.usage.clone();
            }
            RoderEvent::TurnInterrupted(event) => {
                let turn =
                    ensure_turn_record(&mut turns, thread_id, &event.turn_id, event.timestamp);
                turn.completed_at = Some(event.timestamp);
            }
            _ => continue,
        }
    }
    turns
}

fn ensure_turn_record<'a>(
    turns: &'a mut Vec<TurnRecord>,
    thread_id: &ThreadId,
    turn_id: &str,
    created_at: OffsetDateTime,
) -> &'a mut TurnRecord {
    if let Some(index) = turns.iter().position(|turn| turn.turn_id == turn_id) {
        return &mut turns[index];
    }
    turns.push(TurnRecord {
        thread_id: thread_id.clone(),
        turn_id: turn_id.to_string(),
        items: Vec::new(),
        created_at,
        completed_at: None,
        usage: None,
    });
    turns.last_mut().expect("turn was just pushed")
}

pub fn project_thread_item_events(events: &[ThreadItemEvent]) -> Vec<ThreadItemTurnRecord> {
    let mut turns = Vec::<ThreadItemTurnRecord>::new();
    for event in events {
        let turn_index = turns
            .iter()
            .position(|turn| turn.turn_id == event.turn_id)
            .unwrap_or_else(|| {
                turns.push(ThreadItemTurnRecord {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    created_at: event.timestamp,
                    items: Vec::new(),
                });
                turns.len() - 1
            });
        apply_thread_item_event(&mut turns[turn_index].items, &event.event);
    }
    turns
}

fn apply_thread_item_event(items: &mut Vec<ThreadItem>, event: &ThreadItemEventKind) {
    match event {
        ThreadItemEventKind::ItemStarted { item } => {
            if !upsert_item_if_missing(items, item.clone()) {
                merge_started_item(items, item);
            }
        }
        ThreadItemEventKind::ItemDelta { item_id, delta } => {
            let index = item_index(items, item_id).unwrap_or_else(|| {
                items.push(item_for_delta(item_id, delta));
                items.len() - 1
            });
            apply_thread_item_delta(&mut items[index], delta);
        }
        ThreadItemEventKind::ItemCompleted { item } => {
            if let Some(index) = item_index(items, item.id()) {
                merge_completed_item(&mut items[index], item.clone());
            } else {
                items.push(item.clone());
            }
        }
    }
}

fn upsert_item_if_missing(items: &mut Vec<ThreadItem>, item: ThreadItem) -> bool {
    if item_index(items, item.id()).is_some() {
        return false;
    }
    items.push(item);
    true
}

fn merge_started_item(items: &mut [ThreadItem], incoming: &ThreadItem) {
    if let Some(index) = item_index(items, incoming.id()) {
        merge_completed_item(&mut items[index], incoming.clone());
    }
}

fn item_index(items: &[ThreadItem], item_id: &str) -> Option<usize> {
    items.iter().position(|item| item.id() == item_id)
}

fn item_for_delta(item_id: &str, delta: &ThreadItemDelta) -> ThreadItem {
    match delta {
        ThreadItemDelta::AgentMessageText { phase, .. } => ThreadItem::AgentMessage {
            id: item_id.to_string(),
            text: String::new(),
            phase: phase.clone(),
            status: Some(ThreadItemStatus::InProgress),
        },
        ThreadItemDelta::ReasoningText { content_index, .. } => ThreadItem::Reasoning {
            id: item_id.to_string(),
            summary: Vec::new(),
            content: vec![String::new(); content_index + 1],
            status: Some(ThreadItemStatus::InProgress),
        },
        ThreadItemDelta::ReasoningSummaryPartAdded { summary_index }
        | ThreadItemDelta::ReasoningSummaryText { summary_index, .. } => ThreadItem::Reasoning {
            id: item_id.to_string(),
            summary: vec![String::new(); summary_index + 1],
            content: Vec::new(),
            status: Some(ThreadItemStatus::InProgress),
        },
    }
}

fn apply_thread_item_delta(item: &mut ThreadItem, delta: &ThreadItemDelta) {
    match (item, delta) {
        (
            ThreadItem::AgentMessage {
                text,
                phase,
                status,
                ..
            },
            ThreadItemDelta::AgentMessageText {
                delta,
                phase: delta_phase,
            },
        ) => {
            text.push_str(delta);
            if phase.is_none() {
                *phase = delta_phase.clone();
            }
            *status = Some(ThreadItemStatus::InProgress);
        }
        (
            ThreadItem::Reasoning {
                content, status, ..
            },
            ThreadItemDelta::ReasoningText {
                delta,
                content_index,
            },
        ) => {
            ensure_vec_slot(content, *content_index);
            content[*content_index].push_str(delta);
            *status = Some(ThreadItemStatus::InProgress);
        }
        (
            ThreadItem::Reasoning {
                summary, status, ..
            },
            ThreadItemDelta::ReasoningSummaryPartAdded { summary_index },
        ) => {
            ensure_vec_slot(summary, *summary_index);
            *status = Some(ThreadItemStatus::InProgress);
        }
        (
            ThreadItem::Reasoning {
                summary, status, ..
            },
            ThreadItemDelta::ReasoningSummaryText {
                delta,
                summary_index,
            },
        ) => {
            ensure_vec_slot(summary, *summary_index);
            summary[*summary_index].push_str(delta);
            *status = Some(ThreadItemStatus::InProgress);
        }
        (item, delta) => {
            *item = item_for_delta(item.id(), delta);
            apply_thread_item_delta(item, delta);
        }
    }
}

fn ensure_vec_slot(values: &mut Vec<String>, index: usize) {
    while values.len() <= index {
        values.push(String::new());
    }
}

fn merge_completed_item(existing: &mut ThreadItem, incoming: ThreadItem) {
    match (&mut *existing, incoming) {
        (
            ThreadItem::Reasoning {
                summary,
                content,
                status,
                ..
            },
            ThreadItem::Reasoning {
                summary: incoming_summary,
                content: incoming_content,
                status: incoming_status,
                ..
            },
        ) => {
            if !incoming_summary.is_empty() {
                *summary = incoming_summary;
            }
            if !incoming_content.is_empty() {
                *content = incoming_content;
            }
            *status = incoming_status.or(Some(ThreadItemStatus::Completed));
        }
        (
            ThreadItem::ToolExecution {
                status,
                input,
                output,
                error,
                ..
            },
            ThreadItem::ToolExecution {
                status: incoming_status,
                input: incoming_input,
                output: incoming_output,
                error: incoming_error,
                ..
            },
        ) => {
            *status = incoming_status;
            if incoming_input.is_some() {
                *input = incoming_input;
            }
            if incoming_output.is_some() {
                *output = incoming_output;
            }
            if incoming_error.is_some() {
                *error = incoming_error;
            }
        }
        (slot, incoming) => *slot = incoming,
    }
}
