use roder_api::thread::{
    project_thread_item_events, ThreadItem, ThreadItemStatus, ThreadItemTurnRecord,
};
use roder_api::transcript::{InputImage, TranscriptItem};
use roder_protocol::{Thread, ThreadStatus, Turn, TurnInputItem};

pub(crate) fn protocol_thread_from_metadata(
    metadata: roder_api::thread::ThreadMetadata,
    turns: Option<Vec<Turn>>,
    status: ThreadStatus,
) -> Thread {
    let preview = metadata
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "Untitled thread".to_string());
    Thread {
        id: metadata.thread_id.clone(),
        preview,
        model_provider: metadata.provider.unwrap_or_else(|| "mock".to_string()),
        model: metadata.model.unwrap_or_else(|| "mock".to_string()),
        created_at: metadata.created_at.unix_timestamp(),
        updated_at: metadata.updated_at.unix_timestamp(),
        status,
        cwd: metadata.workspace,
        workspace_id: metadata.workspace_id,
        root_id: metadata.root_id,
        name: metadata.title,
        turns,
        usage: metadata.usage,
    }
}

pub(crate) fn idle_thread_status() -> ThreadStatus {
    ThreadStatus {
        kind: "idle".to_string(),
        active_turn_id: None,
        active_flags: Vec::new(),
    }
}

pub(crate) fn running_thread_status(turn_id: String, active_flags: Vec<String>) -> ThreadStatus {
    ThreadStatus {
        kind: "running".to_string(),
        active_turn_id: Some(turn_id),
        active_flags,
    }
}

pub(crate) fn thread_status_for_activity(
    active_turn_id: Option<String>,
    active_flags: Vec<String>,
) -> ThreadStatus {
    match active_turn_id {
        Some(turn_id) => running_thread_status(turn_id, active_flags),
        None => idle_thread_status(),
    }
}

pub(crate) fn protocol_turns_from_snapshot(
    snapshot: &roder_api::thread::ThreadSnapshot,
) -> Vec<Turn> {
    let item_turns = project_thread_item_events(&snapshot.item_events);
    let mut turns = snapshot
        .turns
        .iter()
        .map(|record| {
            let projected_items = item_turns
                .iter()
                .find(|turn| turn.turn_id == record.turn_id)
                .map(|turn| turn.items.clone());
            let items = if record.completed_at.is_some() {
                thread_items_from_transcript_items(&record.turn_id, &record.items)
            } else {
                projected_items.unwrap_or_else(|| {
                    thread_items_from_transcript_items(&record.turn_id, &record.items)
                })
            };
            protocol_turn_from_items(record, items)
        })
        .collect::<Vec<_>>();
    for item_turn in item_turns.iter().filter(|item_turn| {
        !snapshot
            .turns
            .iter()
            .any(|record| record.turn_id == item_turn.turn_id)
    }) {
        turns.push(protocol_turn_from_item_turn(item_turn));
    }
    turns
}

fn thread_items_from_transcript_items(turn_id: &str, items: &[TranscriptItem]) -> Vec<ThreadItem> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| thread_item_from_transcript_item(turn_id, index, item))
        .collect()
}

fn thread_item_from_transcript_item(
    turn_id: &str,
    index: usize,
    item: &TranscriptItem,
) -> ThreadItem {
    match item {
        TranscriptItem::UserMessage(message) => ThreadItem::UserMessage {
            id: format!("{turn_id}-user-{index}"),
            text: message.text.clone(),
            images: message.images.clone(),
            status: Some(ThreadItemStatus::Completed),
        },
        TranscriptItem::AssistantMessage(message) => ThreadItem::AgentMessage {
            id: format!(
                "{turn_id}-agent-{}-{index}",
                message.phase.as_deref().unwrap_or("final_answer")
            ),
            text: message.text.clone(),
            phase: message.phase.clone(),
            status: Some(ThreadItemStatus::Completed),
        },
        TranscriptItem::ReasoningSummary(summary) => ThreadItem::Reasoning {
            id: format!("{turn_id}-reasoning-{index}"),
            summary: Vec::new(),
            content: vec![summary.text.clone()],
            status: Some(ThreadItemStatus::Completed),
        },
        TranscriptItem::ToolCall(call) => ThreadItem::ToolExecution {
            id: call.id.clone(),
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            status: ThreadItemStatus::InProgress,
            input: serde_json::from_str(&call.arguments).ok(),
            output: None,
            error: None,
        },
        TranscriptItem::ToolResult(result) => ThreadItem::ToolExecution {
            id: result.id.clone(),
            tool_call_id: result.id.clone(),
            tool_name: result.name.clone().unwrap_or_else(|| "tool".to_string()),
            status: if result.is_error {
                ThreadItemStatus::Failed
            } else {
                ThreadItemStatus::Completed
            },
            input: result.display_payload.clone(),
            output: (!result.is_error).then(|| result.result.clone()),
            error: result.is_error.then(|| result.result.clone()),
        },
        TranscriptItem::ContextCompaction(compaction) => ThreadItem::Compaction {
            id: format!("{turn_id}-compaction-{index}"),
            summary: compaction.summary.clone(),
            status: Some(ThreadItemStatus::Completed),
        },
        TranscriptItem::Error(error) => ThreadItem::Error {
            id: format!("{turn_id}-error-{index}"),
            message: error.message.clone(),
            status: Some(ThreadItemStatus::Failed),
        },
        other => ThreadItem::Raw {
            id: format!("{turn_id}-item-{index}"),
            payload: serde_json::to_value(other).unwrap_or(serde_json::Value::Null),
            status: Some(ThreadItemStatus::Completed),
        },
    }
}

fn protocol_turn_from_item_turn(record: &ThreadItemTurnRecord) -> Turn {
    Turn {
        id: record.turn_id.clone(),
        items: record.items.clone().into_iter().map(Into::into).collect(),
        items_view: "default".to_string(),
        status: "inProgress".to_string(),
        error: None,
        started_at: Some(record.created_at.unix_timestamp()),
        completed_at: None,
        duration_ms: None,
        usage: None,
    }
}

fn protocol_turn_from_items(
    record: &roder_api::thread::TurnRecord,
    items: Vec<ThreadItem>,
) -> Turn {
    let status = if record.completed_at.is_some() {
        "completed"
    } else {
        "inProgress"
    }
    .to_string();
    let duration_ms = record
        .completed_at
        .map(|completed| (completed - record.created_at).whole_milliseconds().max(0) as i64);
    Turn {
        id: record.turn_id.clone(),
        items: items.into_iter().map(Into::into).collect(),
        items_view: "default".to_string(),
        status,
        error: None,
        started_at: Some(record.created_at.unix_timestamp()),
        completed_at: record.completed_at.map(|time| time.unix_timestamp()),
        duration_ms,
        usage: record.usage.clone(),
    }
}

pub(crate) fn protocol_turn_message(input: &[TurnInputItem], prompt: Option<String>) -> String {
    let text = input
        .iter()
        .filter(|item| item.kind == "text")
        .filter_map(|item| item.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        prompt.unwrap_or_default()
    } else {
        text
    }
}

pub(crate) fn protocol_turn_images(input: &[TurnInputItem]) -> Vec<InputImage> {
    input
        .iter()
        .filter(|item| matches!(item.kind.as_str(), "image" | "input_image"))
        .filter_map(|item| item.image_url.as_ref())
        .map(|image_url| InputImage {
            image_url: image_url.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::thread::{
        ThreadItemDelta, ThreadItemEvent, ThreadItemEventKind, ThreadSnapshot, TurnRecord,
    };
    use roder_api::transcript::{AssistantMessage, ToolCallRecord, UserMessage};
    use roder_protocol::{Item, ThreadItemStatus};
    use time::OffsetDateTime;

    #[test]
    fn snapshot_projection_includes_active_item_event_turns_without_legacy_rows() {
        let timestamp = OffsetDateTime::UNIX_EPOCH;
        let turns = protocol_turns_from_snapshot(&ThreadSnapshot {
            metadata: None,
            events: Vec::new(),
            turns: Vec::new(),
            item_events: vec![ThreadItemEvent {
                seq: 1,
                event_id: "item-event-1".to_string(),
                thread_id: "thread-1".to_string(),
                turn_id: "turn-active".to_string(),
                timestamp,
                event: ThreadItemEventKind::ItemDelta {
                    item_id: "turn-active-agent-reasoning".to_string(),
                    delta: ThreadItemDelta::ReasoningText {
                        delta: "thinking".to_string(),
                        content_index: 0,
                    },
                },
            }],
            extension_states: Vec::new(),
        });

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-active");
        assert_eq!(turns[0].status, "inProgress");
        match turns[0].items.first().expect("active reasoning item") {
            Item::Reasoning {
                id,
                content,
                status,
                ..
            } => {
                assert_eq!(id, "turn-active-agent-reasoning");
                assert_eq!(content, &vec!["thinking".to_string()]);
                assert_eq!(status, &Some(ThreadItemStatus::InProgress));
            }
            other => panic!("expected active reasoning item, got {other:?}"),
        }
    }

    #[test]
    fn completed_snapshot_uses_transcript_rows_to_preserve_intermediary_commentary() {
        let timestamp = OffsetDateTime::UNIX_EPOCH;
        let turns = protocol_turns_from_snapshot(&ThreadSnapshot {
            metadata: None,
            events: Vec::new(),
            turns: vec![TurnRecord {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                items: vec![
                    TranscriptItem::UserMessage(UserMessage::text("inspect")),
                    TranscriptItem::AssistantMessage(AssistantMessage {
                        text: "First commentary.".to_string(),
                        phase: Some("commentary".to_string()),
                    }),
                    TranscriptItem::ToolCall(ToolCallRecord {
                        id: "tool-1".to_string(),
                        name: "read_file".to_string(),
                        arguments: r#"{"path":"README.md"}"#.to_string(),
                    }),
                    TranscriptItem::AssistantMessage(AssistantMessage {
                        text: "Second commentary.".to_string(),
                        phase: Some("commentary".to_string()),
                    }),
                ],
                created_at: timestamp,
                completed_at: Some(timestamp),
                usage: None,
            }],
            item_events: vec![
                ThreadItemEvent {
                    seq: 1,
                    event_id: "item-event-1".to_string(),
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-1".to_string(),
                    timestamp,
                    event: ThreadItemEventKind::ItemDelta {
                        item_id: "turn-1-agent-commentary".to_string(),
                        delta: ThreadItemDelta::AgentMessageText {
                            delta: "First commentary.".to_string(),
                            phase: Some("commentary".to_string()),
                        },
                    },
                },
                ThreadItemEvent {
                    seq: 2,
                    event_id: "item-event-2".to_string(),
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-1".to_string(),
                    timestamp,
                    event: ThreadItemEventKind::ItemDelta {
                        item_id: "turn-1-agent-commentary".to_string(),
                        delta: ThreadItemDelta::AgentMessageText {
                            delta: "Second commentary.".to_string(),
                            phase: Some("commentary".to_string()),
                        },
                    },
                },
            ],
            extension_states: Vec::new(),
        });

        assert_eq!(turns.len(), 1);
        let commentary = turns[0]
            .items
            .iter()
            .filter_map(|item| match item {
                Item::AgentMessage { text, phase, .. }
                    if phase.as_deref() == Some("commentary") =>
                {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(commentary, vec!["First commentary.", "Second commentary."]);
    }
}
