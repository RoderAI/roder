use roder_api::thread::{ThreadItem, ThreadItemStatus};
use roder_api::transcript::TranscriptItem;

pub(super) fn convert(turn: &str, offset: u64, item: &TranscriptItem) -> ThreadItem {
    let completed = Some(ThreadItemStatus::Completed);
    match item {
        TranscriptItem::UserMessage(message) => ThreadItem::UserMessage {
            id: format!("{turn}-user-{offset}"),
            text: message.text.clone(),
            images: message.images.clone(),
            status: completed,
        },
        TranscriptItem::AssistantMessage(message) => ThreadItem::AgentMessage {
            id: format!(
                "{turn}-agent-{}-{offset}",
                message.phase.as_deref().unwrap_or("final_answer")
            ),
            text: message.text.clone(),
            phase: message.phase.clone(),
            status: completed,
        },
        TranscriptItem::ReasoningSummary(summary) => ThreadItem::Reasoning {
            id: format!("{turn}-reasoning-{offset}"),
            summary: Vec::new(),
            content: vec![summary.text.clone()],
            status: completed,
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
            tool_name: result.name.clone().unwrap_or_else(|| "tool".into()),
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
            id: format!("{turn}-compaction-{offset}"),
            summary: compaction.summary.clone(),
            status: completed,
        },
        TranscriptItem::Error(error) => ThreadItem::Error {
            id: format!("{turn}-error-{offset}"),
            message: error.message.clone(),
            status: Some(ThreadItemStatus::Failed),
        },
        other => ThreadItem::Raw {
            id: format!("{turn}-item-{offset}"),
            payload: serde_json::to_value(other).unwrap_or_default(),
            status: completed,
        },
    }
}
