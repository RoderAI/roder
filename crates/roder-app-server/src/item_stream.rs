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
            let item_id = if matches!(event.event, InferenceEvent::ReasoningDelta(_)) {
                reasoning_item_id_for_delta(runtime, &event.thread_id, &event.turn_id).await?
            } else {
                let Some(item_id) = item_id_for_inference_event(&event.turn_id, &event.event)
                else {
                    return Ok(None);
                };
                item_id
            };
            let item_exists = runtime
                .thread_item_exists(&event.thread_id, &event.turn_id, &item_id)
                .await?;
            let kinds = item_event_kinds_from_inference_event(&event.event, &item_id, item_exists);
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
        RoderEvent::InferenceRoutingDecision(event) => Ok(Some((
            event.thread_id.clone(),
            event.turn_id.clone(),
            event.timestamp,
            vec![ThreadItemEventKind::ItemCompleted {
                item: ThreadItem::RoutingDecision {
                    id: routing_decision_item_id(&event.turn_id, event.round_index),
                    decision: event.clone(),
                    status: Some(ThreadItemStatus::Completed),
                },
            }],
        ))),
        RoderEvent::ContextCompactionStarted(event) => Ok(Some((
            event.thread_id.clone(),
            event.turn_id.clone(),
            event.timestamp,
            vec![ThreadItemEventKind::ItemStarted {
                item: ThreadItem::Compaction {
                    id: compaction_item_id(&event.turn_id),
                    summary: format!(
                        "Compacting {} prior items (~{} tokens)...",
                        event.original_item_count, event.original_estimated_tokens
                    ),
                    status: Some(ThreadItemStatus::InProgress),
                },
            }],
        ))),
        _ => Ok(None),
    }
}

async fn reasoning_item_id_for_delta(
    runtime: &Runtime,
    thread_id: &ThreadId,
    turn_id: &TurnId,
) -> anyhow::Result<String> {
    if let Some(item_id) = runtime
        .current_reasoning_item_id(thread_id, turn_id)
        .await?
    {
        return Ok(item_id);
    }

    let base_id = agent_message_item_id(turn_id, Some("reasoning"));
    if !runtime
        .thread_item_exists(thread_id, turn_id, &base_id)
        .await?
    {
        return Ok(base_id);
    }

    for segment in 2.. {
        let item_id = format!("{base_id}-{segment}");
        if !runtime
            .thread_item_exists(thread_id, turn_id, &item_id)
            .await?
        {
            return Ok(item_id);
        }
    }
    unreachable!("unbounded reasoning segment ids should always have a next value")
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
    event: &InferenceEvent,
    item_id: &str,
    item_exists: bool,
) -> Vec<ThreadItemEventKind> {
    match event {
        InferenceEvent::MessageDelta(delta) => {
            let mut events = Vec::new();
            if !item_exists {
                events.push(ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::AgentMessage {
                        id: item_id.to_string(),
                        text: String::new(),
                        phase: delta.phase.clone(),
                        status: Some(ThreadItemStatus::InProgress),
                    },
                });
            }
            events.push(ThreadItemEventKind::ItemDelta {
                item_id: item_id.to_string(),
                delta: ThreadItemDelta::AgentMessageText {
                    delta: delta.text.clone(),
                    phase: delta.phase.clone(),
                },
            });
            events
        }
        InferenceEvent::ReasoningDelta(delta) => {
            let mut events = Vec::new();
            if !item_exists {
                events.push(ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::Reasoning {
                        id: item_id.to_string(),
                        summary: Vec::new(),
                        content: vec![String::new()],
                        status: Some(ThreadItemStatus::InProgress),
                    },
                });
            }
            events.push(ThreadItemEventKind::ItemDelta {
                item_id: item_id.to_string(),
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
            id: compaction_item_id(turn_id),
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

fn compaction_item_id(turn_id: &str) -> String {
    format!("{turn_id}-compaction")
}

fn routing_decision_item_id(turn_id: &str, round_index: u32) -> String {
    format!("{turn_id}-routing-decision-{round_index}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::{
        ContextCompactionStarted, EventSource, InferenceEventReceived,
        InferenceRoutingDecisionEvent,
    };
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_api::inference::{
        HostedToolCallStarted, MessageDelta, ModelSelection, ReasoningDelta, ToolCallCompleted,
    };
    use roder_api::inference_routing::InferenceRoutingDecision;
    use roder_api::transcript::{ContextCompactionRecord, ToolCallRecord, ToolResultRecord};
    use roder_core::{RuntimeConfig, fake_provider::FakeInferenceEngine};
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn context_compaction_started_projects_in_progress_item() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap();
        let timestamp = OffsetDateTime::UNIX_EPOCH;
        let event = EventEnvelope {
            event_id: "event-1".to_string(),
            seq: 1,
            timestamp,
            source: EventSource::Core,
            kind: "context.compaction_started".to_string(),
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            event: RoderEvent::ContextCompactionStarted(ContextCompactionStarted {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                original_item_count: 42,
                original_estimated_tokens: 1234,
                timestamp,
            }),
        };

        let (_, _, _, events) = item_event_kinds_for_event(&runtime, &event)
            .await
            .unwrap()
            .expect("compaction start projects to item event");

        assert_eq!(events.len(), 1);
        match &events[0] {
            ThreadItemEventKind::ItemStarted {
                item:
                    ThreadItem::Compaction {
                        id,
                        summary,
                        status,
                    },
            } => {
                assert_eq!(id, "turn-1-compaction");
                assert!(summary.contains("Compacting 42 prior items"));
                assert_eq!(status, &Some(ThreadItemStatus::InProgress));
            }
            other => panic!("expected in-progress compaction item, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn inference_routing_decision_projects_completed_item() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap();
        let timestamp = OffsetDateTime::UNIX_EPOCH;
        let selected = ModelSelection {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-5".to_string(),
        };
        let event = EventEnvelope {
            event_id: "event-1".to_string(),
            seq: 1,
            timestamp,
            source: EventSource::Core,
            kind: "inference.routing_decision".to_string(),
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            event: RoderEvent::InferenceRoutingDecision(InferenceRoutingDecisionEvent {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                round_index: 0,
                default_selection: ModelSelection {
                    provider: "openai".to_string(),
                    model: "gpt-5.5".to_string(),
                },
                selected_selection: selected.clone(),
                decision: InferenceRoutingDecision::selected(
                    "local",
                    selected,
                    "Large diff and failing tests",
                ),
                timestamp,
            }),
        };

        let (_, _, _, events) = item_event_kinds_for_event(&runtime, &event)
            .await
            .unwrap()
            .expect("routing decision projects to item event");

        assert_eq!(events.len(), 1);
        match &events[0] {
            ThreadItemEventKind::ItemCompleted {
                item:
                    ThreadItem::RoutingDecision {
                        id,
                        decision,
                        status,
                    },
            } => {
                assert_eq!(id, "turn-1-routing-decision-0");
                assert_eq!(decision.turn_id, "turn-1");
                assert_eq!(decision.selected_selection.model, "claude-sonnet-5");
                assert_eq!(status, &Some(ThreadItemStatus::Completed));
            }
            other => panic!("expected routing decision item, got {other:?}"),
        }
    }

    #[test]
    fn context_compaction_transcript_item_completes_same_item() {
        let event = TranscriptItemAppended {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_type: "context_compaction".to_string(),
            item_index: Some(7),
            item: Some(TranscriptItem::ContextCompaction(ContextCompactionRecord {
                summary: "Previous transcript was compacted.".to_string(),
            })),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        };
        let item = event.item.as_ref().unwrap();

        let projected = item_event_kind_from_transcript_item(&event, 7, item);

        match projected {
            ThreadItemEventKind::ItemCompleted {
                item:
                    ThreadItem::Compaction {
                        id,
                        summary,
                        status,
                    },
            } => {
                assert_eq!(id, "turn-1-compaction");
                assert_eq!(summary, "Previous transcript was compacted.");
                assert_eq!(status, Some(ThreadItemStatus::Completed));
            }
            other => panic!("expected completed compaction item, got {other:?}"),
        }
    }

    #[test]
    fn reasoning_delta_starts_reasoning_item_before_delta() {
        let events = item_event_kinds_from_inference_event(
            &InferenceEvent::ReasoningDelta(ReasoningDelta {
                text: "thinking".to_string(),
            }),
            "turn-1-agent-reasoning",
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

    #[tokio::test]
    async fn reasoning_delta_after_tool_event_starts_new_reasoning_item() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap();
        let thread_id = "thread-1".to_string();
        let turn_id = "turn-1".to_string();
        let timestamp = OffsetDateTime::UNIX_EPOCH;

        runtime
            .record_thread_item_event_kind(
                &thread_id,
                &turn_id,
                timestamp,
                ThreadItemEventKind::ItemDelta {
                    item_id: "turn-1-agent-reasoning".to_string(),
                    delta: ThreadItemDelta::ReasoningText {
                        delta: "first".to_string(),
                        content_index: 0,
                    },
                },
            )
            .await
            .unwrap();
        runtime
            .record_thread_item_event_kind(
                &thread_id,
                &turn_id,
                timestamp,
                ThreadItemEventKind::ItemCompleted {
                    item: ThreadItem::ToolExecution {
                        id: "tool-1".to_string(),
                        tool_call_id: "tool-1".to_string(),
                        tool_name: "read_file".to_string(),
                        status: ThreadItemStatus::Completed,
                        input: None,
                        output: Some("done".to_string()),
                        error: None,
                    },
                },
            )
            .await
            .unwrap();

        let event = EventEnvelope {
            event_id: "event-1".to_string(),
            seq: 3,
            timestamp,
            source: EventSource::Core,
            kind: "inference.event_received".to_string(),
            thread_id: Some(thread_id.clone()),
            turn_id: Some(turn_id.clone()),
            event: RoderEvent::InferenceEventReceived(InferenceEventReceived {
                thread_id,
                turn_id: turn_id.clone(),
                event: InferenceEvent::ReasoningDelta(ReasoningDelta {
                    text: "second".to_string(),
                }),
                timestamp,
            }),
        };

        let (_, _, _, events) = item_event_kinds_for_event(&runtime, &event)
            .await
            .unwrap()
            .expect("reasoning event projects to item events");

        assert_eq!(
            events,
            vec![
                ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::Reasoning {
                        id: "turn-1-agent-reasoning-2".to_string(),
                        summary: Vec::new(),
                        content: vec![String::new()],
                        status: Some(ThreadItemStatus::InProgress),
                    },
                },
                ThreadItemEventKind::ItemDelta {
                    item_id: "turn-1-agent-reasoning-2".to_string(),
                    delta: ThreadItemDelta::ReasoningText {
                        delta: "second".to_string(),
                        content_index: 0,
                    },
                },
            ]
        );
    }

    #[test]
    fn message_delta_starts_agent_item_before_delta() {
        let events = item_event_kinds_from_inference_event(
            &InferenceEvent::MessageDelta(MessageDelta {
                text: "hello".to_string(),
                phase: Some("final_answer".to_string()),
            }),
            "turn-1-agent-final_answer",
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
            &InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "tool-1".to_string(),
                name: "shell".to_string(),
                arguments: "{}".to_string(),
            }),
            "tool-1",
            false,
        );

        assert!(events.is_empty());
    }

    #[test]
    fn hosted_tool_start_can_be_public_item_event() {
        let events = item_event_kinds_from_inference_event(
            &InferenceEvent::HostedToolCallStarted(HostedToolCallStarted {
                id: "search-1".to_string(),
                name: "web_search".to_string(),
            }),
            "search-1",
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
