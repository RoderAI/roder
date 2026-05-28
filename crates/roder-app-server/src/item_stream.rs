use roder_api::events::{EventEnvelope, RoderEvent, ThreadId, TranscriptItemAppended, TurnId};
use roder_api::inference::InferenceEvent;
use roder_api::thread::{ThreadItem, ThreadItemDelta, ThreadItemEventKind, ThreadItemStatus};
use roder_api::transcript::TranscriptItem;
use roder_core::Runtime;
use roder_protocol::JsonRpcNotification;
use time::OffsetDateTime;

use crate::notifications;

pub(crate) async fn item_stream_notifications_for_event(
    runtime: &Runtime,
    envelope: &EventEnvelope,
) -> anyhow::Result<Vec<JsonRpcNotification>> {
    let Some((thread_id, turn_id, timestamp, kinds)) =
        item_event_kinds_for_event(runtime, envelope).await?
    else {
        return Ok(Vec::new());
    };

    let mut notifications_out = Vec::new();
    for kind in kinds {
        let recorded = runtime
            .record_thread_item_event_kind(&thread_id, &turn_id, timestamp, kind)
            .await?;
        if let Some(notification) = notifications::thread_item_event_notification(&recorded) {
            notifications_out.push(notification);
        }
    }
    Ok(notifications_out)
}

async fn item_event_kinds_for_event(
    runtime: &Runtime,
    envelope: &EventEnvelope,
) -> anyhow::Result<Option<(ThreadId, TurnId, OffsetDateTime, Vec<ThreadItemEventKind>)>> {
    match &envelope.event {
        RoderEvent::InferenceEventReceived(event) => {
            let Some(item_id) = item_id_for_inference_event(&event.turn_id, &event.event) else {
                return Ok(None);
            };
            let item_exists = runtime
                .thread_item_exists(&event.thread_id, &event.turn_id, &item_id)
                .await?;
            let kinds =
                item_event_kinds_from_inference_event(&event.turn_id, &event.event, item_exists);
            Ok(Some((
                event.thread_id.clone(),
                event.turn_id.clone(),
                event.timestamp,
                kinds,
            )))
        }
        RoderEvent::TranscriptItemAppended(event) => {
            let Some(item) = &event.item else {
                return Ok(None);
            };
            let item_index = if let Some(item_index) = event.item_index {
                item_index
            } else {
                runtime
                    .latest_transcript_item_index(&event.thread_id, &event.turn_id)
                    .await?
                    .unwrap_or(0)
            };
            Ok(Some((
                event.thread_id.clone(),
                event.turn_id.clone(),
                event.timestamp,
                vec![item_event_kind_from_transcript_item(
                    event, item_index, item,
                )],
            )))
        }
        _ => Ok(None),
    }
}

fn item_id_for_inference_event(turn_id: &str, event: &InferenceEvent) -> Option<String> {
    match event {
        InferenceEvent::MessageDelta(delta) => {
            Some(agent_message_item_id(turn_id, delta.phase.as_deref()))
        }
        InferenceEvent::ReasoningDelta(_) => {
            Some(agent_message_item_id(turn_id, Some("reasoning")))
        }
        InferenceEvent::HostedToolCallStarted(call) => Some(call.id.clone()),
        InferenceEvent::HostedToolCallCompleted(call) => Some(call.id.clone()),
        _ => None,
    }
}

fn item_event_kinds_from_inference_event(
    turn_id: &str,
    event: &InferenceEvent,
    item_exists: bool,
) -> Vec<ThreadItemEventKind> {
    match event {
        InferenceEvent::MessageDelta(delta) => {
            let item_id = agent_message_item_id(turn_id, delta.phase.as_deref());
            let mut events = Vec::new();
            if !item_exists {
                events.push(ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::AgentMessage {
                        id: item_id.clone(),
                        text: String::new(),
                        phase: delta.phase.clone(),
                        status: Some(ThreadItemStatus::InProgress),
                    },
                });
            }
            events.push(ThreadItemEventKind::ItemDelta {
                item_id,
                delta: ThreadItemDelta::AgentMessageText {
                    delta: delta.text.clone(),
                    phase: delta.phase.clone(),
                },
            });
            events
        }
        InferenceEvent::ReasoningDelta(delta) => {
            let item_id = agent_message_item_id(turn_id, Some("reasoning"));
            let mut events = Vec::new();
            if !item_exists {
                events.push(ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::Reasoning {
                        id: item_id.clone(),
                        summary: Vec::new(),
                        content: vec![String::new()],
                        status: Some(ThreadItemStatus::InProgress),
                    },
                });
            }
            events.push(ThreadItemEventKind::ItemDelta {
                item_id,
                delta: ThreadItemDelta::ReasoningText {
                    delta: delta.text.clone(),
                    content_index: 0,
                },
            });
            events
        }
        InferenceEvent::HostedToolCallStarted(call) => {
            if item_exists {
                return Vec::new();
            }
            vec![ThreadItemEventKind::ItemStarted {
                item: ThreadItem::ToolExecution {
                    id: call.id.clone(),
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    status: ThreadItemStatus::InProgress,
                    input: None,
                    output: None,
                    error: None,
                },
            }]
        }
        InferenceEvent::HostedToolCallCompleted(call) => {
            vec![ThreadItemEventKind::ItemCompleted {
                item: ThreadItem::ToolExecution {
                    id: call.id.clone(),
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    status: ThreadItemStatus::Completed,
                    input: serde_json::from_str(&call.arguments).ok(),
                    output: None,
                    error: None,
                },
            }]
        }
        _ => Vec::new(),
    }
}

fn item_event_kind_from_transcript_item(
    event: &TranscriptItemAppended,
    index: usize,
    item: &TranscriptItem,
) -> ThreadItemEventKind {
    let item = thread_item_from_transcript_item(&event.turn_id, index, item);
    match item {
        ThreadItem::ToolExecution {
            status: ThreadItemStatus::InProgress,
            ..
        } => ThreadItemEventKind::ItemStarted { item },
        _ => ThreadItemEventKind::ItemCompleted { item },
    }
}

fn thread_item_from_transcript_item(
    turn_id: &TurnId,
    index: usize,
    item: &TranscriptItem,
) -> ThreadItem {
    match item {
        TranscriptItem::UserMessage(message) => ThreadItem::UserMessage {
            id: format!("{turn_id}-user"),
            text: message.text.clone(),
            images: message.images.clone(),
            status: Some(ThreadItemStatus::Completed),
        },
        TranscriptItem::AssistantMessage(message) => ThreadItem::AgentMessage {
            id: agent_message_item_id(turn_id, message.phase.as_deref()),
            text: message.text.clone(),
            phase: message.phase.clone(),
            status: Some(ThreadItemStatus::Completed),
        },
        TranscriptItem::ReasoningSummary(summary) => ThreadItem::Reasoning {
            id: agent_message_item_id(turn_id, Some("reasoning")),
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

fn agent_message_item_id(turn_id: &str, phase: Option<&str>) -> String {
    format!("{}-agent-{}", turn_id, phase.unwrap_or("final_answer"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::inference::{
        HostedToolCallStarted, MessageDelta, ReasoningDelta, ToolCallCompleted,
    };
    use roder_api::transcript::{ToolCallRecord, ToolResultRecord};
    use serde_json::json;

    #[test]
    fn reasoning_delta_starts_reasoning_item_before_delta() {
        let events = item_event_kinds_from_inference_event(
            "turn-1",
            &InferenceEvent::ReasoningDelta(ReasoningDelta {
                text: "thinking".to_string(),
            }),
            false,
        );

        assert_eq!(
            events,
            vec![
                ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::Reasoning {
                        id: "turn-1-agent-reasoning".to_string(),
                        summary: Vec::new(),
                        content: vec![String::new()],
                        status: Some(ThreadItemStatus::InProgress),
                    },
                },
                ThreadItemEventKind::ItemDelta {
                    item_id: "turn-1-agent-reasoning".to_string(),
                    delta: ThreadItemDelta::ReasoningText {
                        delta: "thinking".to_string(),
                        content_index: 0,
                    },
                },
            ]
        );
    }

    #[test]
    fn message_delta_starts_agent_item_before_delta() {
        let events = item_event_kinds_from_inference_event(
            "turn-1",
            &InferenceEvent::MessageDelta(MessageDelta {
                text: "hello".to_string(),
                phase: Some("final_answer".to_string()),
            }),
            false,
        );

        assert_eq!(
            events,
            vec![
                ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::AgentMessage {
                        id: "turn-1-agent-final_answer".to_string(),
                        text: String::new(),
                        phase: Some("final_answer".to_string()),
                        status: Some(ThreadItemStatus::InProgress),
                    },
                },
                ThreadItemEventKind::ItemDelta {
                    item_id: "turn-1-agent-final_answer".to_string(),
                    delta: ThreadItemDelta::AgentMessageText {
                        delta: "hello".to_string(),
                        phase: Some("final_answer".to_string()),
                    },
                },
            ]
        );
    }

    #[test]
    fn provider_tool_events_are_not_public_item_events() {
        let events = item_event_kinds_from_inference_event(
            "turn-1",
            &InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "tool-1".to_string(),
                name: "shell".to_string(),
                arguments: "{}".to_string(),
            }),
            false,
        );

        assert!(events.is_empty());
    }

    #[test]
    fn hosted_tool_start_can_be_public_item_event() {
        let events = item_event_kinds_from_inference_event(
            "turn-1",
            &InferenceEvent::HostedToolCallStarted(HostedToolCallStarted {
                id: "search-1".to_string(),
                name: "web_search".to_string(),
            }),
            false,
        );

        assert_eq!(
            events,
            vec![ThreadItemEventKind::ItemStarted {
                item: ThreadItem::ToolExecution {
                    id: "search-1".to_string(),
                    tool_call_id: "search-1".to_string(),
                    tool_name: "web_search".to_string(),
                    status: ThreadItemStatus::InProgress,
                    input: None,
                    output: None,
                    error: None,
                },
            }]
        );
    }

    #[test]
    fn transcript_tool_start_and_result_share_tool_execution_id() {
        let timestamp = OffsetDateTime::UNIX_EPOCH;
        let start = TranscriptItem::ToolCall(ToolCallRecord {
            id: "tool-1".to_string(),
            name: "shell".to_string(),
            arguments: "{\"cmd\":\"pwd\"}".to_string(),
        });
        let result = TranscriptItem::ToolResult(ToolResultRecord {
            id: "tool-1".to_string(),
            name: Some("shell".to_string()),
            result: "/tmp".to_string(),
            display_payload: Some(json!({ "cmd": "pwd" })),
            is_error: false,
        });
        let appended = TranscriptItemAppended {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_type: "tool_call".to_string(),
            item_index: Some(1),
            item: Some(start.clone()),
            timestamp,
        };
        let completed = TranscriptItemAppended {
            item_type: "tool_result".to_string(),
            item: Some(result.clone()),
            ..appended.clone()
        };

        assert_eq!(
            item_event_kind_from_transcript_item(&appended, 1, &start),
            ThreadItemEventKind::ItemStarted {
                item: ThreadItem::ToolExecution {
                    id: "tool-1".to_string(),
                    tool_call_id: "tool-1".to_string(),
                    tool_name: "shell".to_string(),
                    status: ThreadItemStatus::InProgress,
                    input: Some(json!({ "cmd": "pwd" })),
                    output: None,
                    error: None,
                },
            }
        );
        assert_eq!(
            item_event_kind_from_transcript_item(&completed, 2, &result),
            ThreadItemEventKind::ItemCompleted {
                item: ThreadItem::ToolExecution {
                    id: "tool-1".to_string(),
                    tool_call_id: "tool-1".to_string(),
                    tool_name: "shell".to_string(),
                    status: ThreadItemStatus::Completed,
                    input: Some(json!({ "cmd": "pwd" })),
                    output: Some("/tmp".to_string()),
                    error: None,
                },
            }
        );
    }
}
