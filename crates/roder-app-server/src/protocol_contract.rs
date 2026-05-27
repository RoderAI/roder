use roder_api::thread::{
    ThreadItem, ThreadItemTurnRecord, project_thread_item_events,
    public_thread_item_from_transcript_item,
};
use roder_api::transcript::InputImage;
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

pub(crate) fn protocol_turn_from_record(record: roder_api::thread::TurnRecord) -> Turn {
    let items = record
        .items
        .into_iter()
        .enumerate()
        .map(|(index, item)| public_thread_item_from_transcript_item(&record.turn_id, index, &item))
        .collect::<Vec<_>>();
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
        id: record.turn_id,
        items,
        items_view: "default".to_string(),
        status,
        error: None,
        started_at: Some(record.created_at.unix_timestamp()),
        completed_at: record.completed_at.map(|time| time.unix_timestamp()),
        duration_ms,
        usage: record.usage,
    }
}

pub(crate) fn protocol_turns_from_snapshot(
    snapshot: &roder_api::thread::ThreadSnapshot,
) -> Vec<Turn> {
    if !snapshot.item_events.is_empty() {
        let item_turns = project_thread_item_events(&snapshot.item_events);
        let mut turns = snapshot
            .turns
            .iter()
            .map(|record| {
                let items = item_turns
                    .iter()
                    .find(|turn| turn.turn_id == record.turn_id)
                    .map(|turn| turn.items.clone())
                    .unwrap_or_else(|| legacy_thread_items(record));
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
        return turns;
    }

    snapshot
        .turns
        .iter()
        .cloned()
        .map(protocol_turn_from_record)
        .collect()
}

fn protocol_turn_from_item_turn(record: &ThreadItemTurnRecord) -> Turn {
    Turn {
        id: record.turn_id.clone(),
        items: record.items.clone(),
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
        items,
        items_view: "default".to_string(),
        status,
        error: None,
        started_at: Some(record.created_at.unix_timestamp()),
        completed_at: record.completed_at.map(|time| time.unix_timestamp()),
        duration_ms,
        usage: record.usage.clone(),
    }
}

fn legacy_thread_items(record: &roder_api::thread::TurnRecord) -> Vec<ThreadItem> {
    record
        .items
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, item)| public_thread_item_from_transcript_item(&record.turn_id, index, &item))
        .collect()
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
        ThreadItemDelta, ThreadItemEvent, ThreadItemEventKind, ThreadItemStatus, ThreadSnapshot,
        TurnRecord,
    };
    use roder_api::transcript::{ToolResultRecord, TranscriptItem};
    use serde_json::json;
    use time::OffsetDateTime;

    #[test]
    fn persisted_tool_result_rehydrates_payload_for_client() {
        let turn = protocol_turn_from_record(TurnRecord {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            items: vec![TranscriptItem::ToolResult(ToolResultRecord {
                id: "tool-1".to_string(),
                name: Some("list_files".to_string()),
                result: "src\nCargo.toml".to_string(),
                display_payload: Some(json!({ "path": ".", "shown": 2 })),
                is_error: false,
            })],
            created_at: OffsetDateTime::UNIX_EPOCH,
            completed_at: Some(OffsetDateTime::UNIX_EPOCH),
            usage: None,
        });

        let item = turn.items.first().expect("tool item");
        match item {
            ThreadItem::ToolExecution {
                tool_name,
                input,
                output,
                ..
            } => {
                assert_eq!(tool_name, "list_files");
                assert_eq!(input.as_ref().unwrap()["path"], ".");
                assert_eq!(input.as_ref().unwrap()["shown"], 2);
                assert_eq!(output.as_deref(), Some("src\nCargo.toml"));
            }
            other => panic!("expected tool execution item, got {other:?}"),
        }
    }

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
            ThreadItem::Reasoning {
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
}
