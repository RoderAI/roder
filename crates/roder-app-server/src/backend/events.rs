use roder_api::backend::BackendEvent;
use roder_api::events::{
    ApprovalRequested, FileChanged, InferenceEventReceived, RoderEvent, ToolCallCompleted,
    ToolCallRequested, ToolOutputDelta, TurnCompleted, TurnFailed, TurnInterrupted, TurnStarted,
    UserInputRequested,
};
use roder_api::inference::{InferenceEvent, MessageDelta, TokenUsage};
use roder_core::Runtime;
use roder_protocol::{Item, Thread, ThreadItemStatus, Turn};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::RwLock;

pub(super) async fn emit_event(
    event: BackendEvent,
    thread_id: &str,
    turn_id: &mut String,
    output: &mut HashMap<String, String>,
    usage: &mut Option<TokenUsage>,
    thread: &Arc<RwLock<Thread>>,
    runtime: &Arc<Runtime>,
    routes: &Arc<RwLock<HashMap<String, String>>>,
) {
    let now = OffsetDateTime::now_utc();
    update_history(&event, thread, turn_id, now).await;
    let id = turn_id.clone();
    let event = match event {
        BackendEvent::TurnStarted { turn_id: started } => {
            *usage = None;
            *turn_id = started.clone();
            {
                let mut thread = thread.write().await;
                thread.status.kind = "active".into();
                thread.status.active_turn_id = Some(started.clone());
            }
            RoderEvent::TurnStarted(TurnStarted {
                thread_id: thread_id.into(),
                turn_id: started,
                runtime_profile: Default::default(),
                timestamp: now,
            })
        }
        BackendEvent::Text(text) => RoderEvent::InferenceEventReceived(InferenceEventReceived {
            thread_id: thread_id.into(),
            turn_id: id,
            event: InferenceEvent::MessageDelta(MessageDelta { text, phase: None }),
            timestamp: now,
        }),
        BackendEvent::Usage(tokens) => {
            *usage = Some(tokens.clone());
            RoderEvent::InferenceEventReceived(InferenceEventReceived {
                thread_id: thread_id.into(),
                turn_id: id,
                event: InferenceEvent::Usage(tokens),
                timestamp: now,
            })
        }
        BackendEvent::ProviderMetadata(metadata) => {
            RoderEvent::InferenceEventReceived(InferenceEventReceived {
                thread_id: thread_id.into(),
                turn_id: id,
                event: InferenceEvent::ProviderMetadata(metadata),
                timestamp: now,
            })
        }
        BackendEvent::ToolStarted { id: tool_id, label } => {
            RoderEvent::ToolCallRequested(ToolCallRequested {
                thread_id: thread_id.into(),
                turn_id: id,
                tool_id,
                tool_name: label,
                display_payload: None,
                timestamp: now,
            })
        }
        BackendEvent::ToolOutput { id: tool_id, text } => {
            output.entry(tool_id.clone()).or_default().push_str(&text);
            runtime
                .emit(RoderEvent::ToolOutputDelta(ToolOutputDelta {
                    thread_id: thread_id.into(),
                    turn_id: id,
                    tool_id,
                    delta: text,
                    timestamp: now,
                }))
                .await;
            return;
        }
        BackendEvent::ToolFinished {
            id: tool_id,
            label,
            summary,
        } => RoderEvent::ToolCallCompleted(ToolCallCompleted {
            thread_id: thread_id.into(),
            turn_id: id,
            tool_id: tool_id.clone(),
            tool_name: Some(label),
            display_payload: None,
            is_error: summary.contains("failed"),
            output: output.remove(&tool_id).or(Some(summary)),
            timestamp: now,
        }),
        BackendEvent::FileChanged { path, change_type } => RoderEvent::FileChanged(FileChanged {
            thread_id: thread_id.into(),
            turn_id: id,
            path,
            change_type,
            timestamp: now,
        }),
        BackendEvent::Approval {
            request_id,
            description,
        } => {
            routes
                .write()
                .await
                .insert(request_id.clone(), thread_id.into());
            RoderEvent::ApprovalRequested(ApprovalRequested {
                thread_id: thread_id.into(),
                turn_id: id,
                approval_id: request_id.clone(),
                tool_id: request_id,
                tool_name: "Codex tool".into(),
                reason: Some(description),
                timestamp: now,
            })
        }
        BackendEvent::Question {
            request_id,
            questions,
        } => {
            routes
                .write()
                .await
                .insert(request_id.clone(), thread_id.into());
            let questions = questions.into_iter().map(|q| json!({
                "id":q.id,"header":"Codex","question":q.text,
                "options":q.options.into_iter().map(|label| json!({"label":label,"description":""})).collect::<Vec<_>>()
            })).collect::<Vec<_>>();
            RoderEvent::UserInputRequested(UserInputRequested {
                thread_id: thread_id.into(),
                turn_id: id,
                request_id,
                questions: json!(questions),
                timestamp: now,
            })
        }
        BackendEvent::TurnFinished {
            turn_id: finished,
            status,
        } => {
            {
                let mut thread = thread.write().await;
                thread.status.kind = "idle".into();
                thread.status.active_turn_id = None;
                if let Some(turn) = thread
                    .turns
                    .as_mut()
                    .and_then(|turns| turns.iter_mut().find(|turn| turn.id == finished))
                {
                    turn.status = status.clone();
                    turn.completed_at = Some(now.unix_timestamp());
                }
            }
            *turn_id = String::new();
            match status.as_str() {
                "interrupted" => RoderEvent::TurnInterrupted(TurnInterrupted {
                    thread_id: thread_id.into(),
                    turn_id: finished,
                    timestamp: now,
                }),
                "completed" => RoderEvent::TurnCompleted(TurnCompleted {
                    thread_id: thread_id.into(),
                    turn_id: finished,
                    usage: usage.take(),
                    finish_reason: None,
                    timestamp: now,
                }),
                _ => RoderEvent::TurnFailed(TurnFailed {
                    thread_id: thread_id.into(),
                    turn_id: finished,
                    error: status,
                    error_kind: Some("backend_failed".into()),
                    usage: usage.take(),
                    timestamp: now,
                }),
            }
        }
        BackendEvent::Notice(message) => {
            if id.is_empty() {
                return;
            }
            RoderEvent::InferenceEventReceived(InferenceEventReceived {
                thread_id: thread_id.into(),
                turn_id: id,
                event: InferenceEvent::MessageDelta(MessageDelta {
                    text: format!("\n{message}\n"),
                    phase: None,
                }),
                timestamp: now,
            })
        }
    };
    runtime.emit(event).await;
}

async fn update_history(
    event: &BackendEvent,
    thread: &Arc<RwLock<Thread>>,
    active_turn_id: &str,
    now: OffsetDateTime,
) {
    let mut thread = thread.write().await;
    let turns = thread.turns.get_or_insert_with(Vec::new);
    if let BackendEvent::TurnStarted { turn_id } = event {
        if !turns.iter().any(|turn| turn.id == *turn_id) {
            turns.push(Turn {
                id: turn_id.clone(),
                items: vec![],
                items_view: "full".into(),
                status: "inProgress".into(),
                error: None,
                started_at: Some(now.unix_timestamp()),
                completed_at: None,
                duration_ms: None,
                usage: None,
                finish_reason: None,
            });
        }
        return;
    }
    let Some(turn) = turns.iter_mut().find(|turn| turn.id == active_turn_id) else {
        return;
    };
    match event {
        BackendEvent::Text(text) => {
            let id = format!("agent-{active_turn_id}");
            if let Some(Item::AgentMessage { text: existing, .. }) = turn.items.iter_mut().find(
                |item| matches!(item, Item::AgentMessage { id: item_id, .. } if *item_id == id),
            ) {
                existing.push_str(text);
            } else {
                turn.items.push(Item::AgentMessage {
                    id,
                    text: text.clone(),
                    phase: None,
                    status: Some(ThreadItemStatus::InProgress),
                });
            }
        }
        BackendEvent::ToolStarted { id, label } => turn.items.push(Item::ToolExecution {
            id: id.clone(),
            tool_call_id: id.clone(),
            tool_name: label.clone(),
            status: ThreadItemStatus::InProgress,
            input: None,
            output: None,
            error: None,
        }),
        BackendEvent::ToolOutput { id, text } => {
            if let Some(Item::ToolExecution { output, .. }) = turn.items.iter_mut().find(
                |item| matches!(item, Item::ToolExecution { id: item_id, .. } if item_id == id),
            ) {
                output.get_or_insert_with(String::new).push_str(text);
            }
        }
        BackendEvent::ToolFinished { id, summary, .. } => {
            if let Some(Item::ToolExecution { status, output, .. }) = turn.items.iter_mut().find(
                |item| matches!(item, Item::ToolExecution { id: item_id, .. } if item_id == id),
            ) {
                *status = if summary.contains("failed") {
                    ThreadItemStatus::Failed
                } else {
                    ThreadItemStatus::Completed
                };
                if output.is_none() {
                    *output = Some(summary.clone());
                }
            }
        }
        BackendEvent::FileChanged { path, change_type } => turn.items.push(Item::Raw {
            id: format!("file-{}", turn.items.len()),
            payload: json!({"type":"fileChange","path":path,"changeType":change_type}),
            status: Some(ThreadItemStatus::Completed),
        }),
        BackendEvent::Usage(usage) => turn.usage = Some(usage.clone()),
        BackendEvent::TurnFinished { status, .. } => {
            turn.status = status.clone();
            turn.completed_at = Some(now.unix_timestamp());
            for item in &mut turn.items {
                if let Item::AgentMessage { status, .. } = item {
                    *status = Some(ThreadItemStatus::Completed);
                }
            }
        }
        _ => {}
    }
}
