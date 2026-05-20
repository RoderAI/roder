use roder_api::conversation::InputImage;
use roder_protocol::{DesktopItem, DesktopThread, DesktopThreadStatus, DesktopTurn, TurnInputItem};

pub(crate) fn desktop_thread_from_metadata(
    metadata: roder_api::session::SessionMetadata,
    turns: Option<Vec<DesktopTurn>>,
) -> DesktopThread {
    let preview = metadata
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "Untitled thread".to_string());
    let cwd = metadata
        .workspace
        .clone()
        .unwrap_or_else(default_cwd_string);
    DesktopThread {
        id: metadata.thread_id.clone(),
        session_id: metadata.thread_id,
        preview,
        model_provider: metadata.provider.unwrap_or_else(|| "mock".to_string()),
        created_at: metadata.created_at.unix_timestamp(),
        updated_at: metadata.updated_at.unix_timestamp(),
        status: DesktopThreadStatus {
            kind: "idle".to_string(),
            active_flags: Vec::new(),
        },
        cwd,
        name: metadata.title,
        turns,
    }
}

pub(crate) fn desktop_turn_from_record(record: roder_api::session::TurnRecord) -> DesktopTurn {
    let items = record
        .items
        .into_iter()
        .enumerate()
        .map(|(index, item)| desktop_item_from_turn_item(record.turn_id.as_str(), index, item))
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
    DesktopTurn {
        id: record.turn_id,
        items,
        items_view: "default".to_string(),
        status,
        error: None,
        started_at: Some(record.created_at.unix_timestamp()),
        completed_at: record.completed_at.map(|time| time.unix_timestamp()),
        duration_ms,
    }
}

fn desktop_item_from_turn_item(
    turn_id: &str,
    index: usize,
    item: roder_api::conversation::TurnItem,
) -> DesktopItem {
    match item {
        roder_api::conversation::ConversationItem::UserMessage(message) => DesktopItem {
            id: format!("{turn_id}-user-{index}"),
            kind: "userMessage".to_string(),
            text: Some(message.text),
            status: Some("completed".to_string()),
            phase: None,
            tool_name: None,
            tool_call_id: None,
            payload: None,
        },
        roder_api::conversation::ConversationItem::AssistantMessage(message) => DesktopItem {
            id: format!("{turn_id}-assistant-{index}"),
            kind: "agentMessage".to_string(),
            text: Some(message.text),
            status: Some("completed".to_string()),
            phase: message.phase,
            tool_name: None,
            tool_call_id: None,
            payload: None,
        },
        roder_api::conversation::ConversationItem::ReasoningSummary(summary) => DesktopItem {
            id: format!("{turn_id}-reasoning-{index}"),
            kind: "reasoning".to_string(),
            text: Some(summary.text),
            status: Some("completed".to_string()),
            phase: Some("reasoning".to_string()),
            tool_name: None,
            tool_call_id: None,
            payload: None,
        },
        roder_api::conversation::ConversationItem::ToolCall(call) => DesktopItem {
            id: call.id.clone(),
            kind: "toolCall".to_string(),
            text: None,
            status: Some("inProgress".to_string()),
            phase: None,
            tool_name: Some(call.name),
            tool_call_id: Some(call.id),
            payload: serde_json::from_str(&call.arguments).ok(),
        },
        roder_api::conversation::ConversationItem::ToolResult(result) => DesktopItem {
            id: result.id.clone(),
            kind: "toolMessage".to_string(),
            text: Some(result.result),
            status: Some(
                if result.is_error {
                    "failed"
                } else {
                    "completed"
                }
                .to_string(),
            ),
            phase: None,
            tool_name: result.name,
            tool_call_id: Some(result.id),
            payload: result.display_payload,
        },
        other => DesktopItem {
            id: format!("{turn_id}-item-{index}"),
            kind: "raw".to_string(),
            text: None,
            status: Some("completed".to_string()),
            phase: None,
            tool_name: None,
            tool_call_id: None,
            payload: Some(serde_json::to_value(other).unwrap_or(serde_json::Value::Null)),
        },
    }
}

pub(crate) fn desktop_turn_message(input: &[TurnInputItem], prompt: Option<String>) -> String {
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

pub(crate) fn desktop_turn_images(input: &[TurnInputItem]) -> Vec<InputImage> {
    input
        .iter()
        .filter(|item| matches!(item.kind.as_str(), "image" | "input_image"))
        .filter_map(|item| item.image_url.as_ref())
        .map(|image_url| InputImage {
            image_url: image_url.clone(),
        })
        .collect()
}

pub(crate) fn default_cwd_string() -> String {
    std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::conversation::{ConversationItem, ToolResultRecord};
    use roder_api::session::TurnRecord;
    use serde_json::json;
    use time::OffsetDateTime;

    #[test]
    fn persisted_tool_result_rehydrates_payload_for_desktop() {
        let turn = desktop_turn_from_record(TurnRecord {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            items: vec![ConversationItem::ToolResult(ToolResultRecord {
                id: "tool-1".to_string(),
                name: Some("list_files".to_string()),
                result: "src\nCargo.toml".to_string(),
                display_payload: Some(json!({ "path": ".", "shown": 2 })),
                is_error: false,
            })],
            created_at: OffsetDateTime::UNIX_EPOCH,
            completed_at: Some(OffsetDateTime::UNIX_EPOCH),
        });

        let item = turn.items.first().expect("tool item");
        assert_eq!(item.kind, "toolMessage");
        assert_eq!(item.tool_name.as_deref(), Some("list_files"));
        assert_eq!(item.payload.as_ref().unwrap()["path"], ".");
        assert_eq!(item.payload.as_ref().unwrap()["shown"], 2);
        assert!(item.payload.as_ref().unwrap().get("input").is_none());
        assert!(item.payload.as_ref().unwrap().get("arguments").is_none());
    }
}
