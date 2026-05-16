use std::collections::HashSet;

use roder_api::catalog::{DEFAULT_MODEL_ID, lookup_model};
use roder_api::conversation::{ConversationItem, ToolCallRecord, ToolResultRecord};
use roder_api::events::{EventEnvelope, RoderEvent, ThreadId, TurnId};
use roder_api::inference::InferenceEvent;
use roder_api::session::{SessionMetadata, ThreadSnapshot, TurnRecord};
use roder_core::{CreateSessionRequest, StartTurnRequest, default_instructions};
use roder_protocol::JsonRpcError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::AppServer;

impl AppServer {
    pub(crate) async fn handle_legacy_initialize(
        &self,
        _params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        Ok(json!({
            "userAgent": "roder-app-server",
            "godeHome": ".roder",
            "platformOs": std::env::consts::OS,
            "cwd": cfg.workspace,
            "capabilities": legacy_capabilities(),
        }))
    }

    pub(crate) async fn handle_legacy_thread_start(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = decode_optional::<LegacyThreadStartParams>(params)?;
        let cfg = self.runtime.status().await;
        let model = trim_nonempty(params.model).unwrap_or(cfg.default_model);
        let provider = trim_nonempty(params.model_provider)
            .or_else(|| provider_for_model(&model).map(str::to_string))
            .unwrap_or(cfg.default_provider);
        let workspace = trim_nonempty(params.cwd).or(cfg.workspace);
        let metadata = self
            .runtime
            .create_session_with(CreateSessionRequest {
                title: None,
                workspace: workspace.clone(),
                provider: Some(provider.clone()),
                model: Some(model.clone()),
            })
            .await
            .map_err(internal_error)?;
        let thread =
            legacy_thread_from_metadata(&metadata, &self.runtime.status().await, Vec::new());
        Ok(json!({
            "thread": thread,
            "model": model,
            "modelProvider": provider,
            "cwd": workspace,
        }))
    }

    pub(crate) async fn handle_legacy_thread_list(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = decode_optional::<LegacyThreadListParams>(params)?;
        let cfg = self.runtime.status().await;
        let mut sessions = self.runtime.list_sessions().await.map_err(internal_error)?;
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
        if let Some(limit) = params.limit.filter(|limit| *limit > 0) {
            sessions.truncate(limit as usize);
        }
        let data = sessions
            .iter()
            .map(|metadata| legacy_thread_from_metadata(metadata, &cfg, Vec::new()))
            .collect::<Vec<_>>();
        Ok(json!({
            "data": data,
            "nextCursor": null,
            "backwardsCursor": null,
        }))
    }

    pub(crate) async fn handle_legacy_thread_read(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = decode_required::<LegacyThreadReadParams>(params)?;
        let snapshot = self
            .runtime
            .load_session(&params.thread_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| invalid_params("thread not found"))?;
        let cfg = self.runtime.status().await;
        let thread = legacy_thread_from_snapshot(snapshot, &cfg, params.include_turns);
        Ok(json!({ "thread": thread }))
    }

    pub(crate) async fn handle_legacy_turn_start(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = decode_required::<LegacyTurnStartParams>(params)?;
        let message = legacy_message_from_turn_params(&params);
        let snapshot = self
            .runtime
            .load_session(&params.thread_id)
            .await
            .map_err(internal_error)?;
        let metadata = snapshot.and_then(|snapshot| snapshot.metadata);
        let turn_id = self
            .runtime
            .start_turn(StartTurnRequest {
                thread_id: params.thread_id,
                message,
                provider_override: metadata
                    .as_ref()
                    .and_then(|metadata| metadata.provider.clone()),
                model_override: metadata
                    .as_ref()
                    .and_then(|metadata| metadata.model.clone()),
                instructions: default_instructions(),
            })
            .await
            .map_err(internal_error)?;
        Ok(json!({ "turnId": turn_id }))
    }

    pub(crate) async fn handle_legacy_turn_steer(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        self.handle_legacy_turn_start(params).await
    }

    pub(crate) async fn handle_legacy_turn_interrupt(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = decode_required::<LegacyTurnInterruptParams>(params)?;
        self.runtime
            .interrupt_turn(params.thread_id, params.turn_id)
            .await
            .map_err(internal_error)?;
        Ok(json!({}))
    }

    pub(crate) async fn handle_legacy_model_list(&self) -> Result<Value, JsonRpcError> {
        let cfg = self.runtime.status().await;
        let mut seen = HashSet::new();
        let mut models = Vec::new();
        for engine in &self.runtime.registry().inference_engines {
            let provider = engine.id();
            let listed = engine
                .list_models(roder_api::inference::InferenceProviderContext {
                    provider_id: &provider,
                })
                .await
                .unwrap_or_default();
            for model in listed {
                if !seen.insert(model.id.clone()) {
                    continue;
                }
                let catalog = lookup_model(&model.id);
                models.push(json!({
                    "id": model.id,
                    "name": model.name,
                    "description": catalog.map(|entry| entry.description),
                    "modelProvider": provider,
                    "reasoningEfforts": catalog
                        .map(|entry| entry.supported_reasoning.iter().map(|reasoning| reasoning.effort).collect::<Vec<_>>())
                        .unwrap_or_default(),
                    "defaultReasoningEffort": catalog.map(|entry| entry.default_reasoning).unwrap_or("none"),
                    "contextWindow": model.context_window.or_else(|| catalog.map(|entry| entry.context_window)),
                    "maxContextWindow": catalog.map(|entry| entry.max_context_window),
                    "supportsImages": catalog.map(|entry| entry.supports_images).unwrap_or(false),
                    "supportsTools": catalog.map(|entry| entry.supports_tools).unwrap_or(false),
                    "supportsStructured": catalog.map(|entry| entry.supports_structured).unwrap_or(false),
                    "editTool": catalog.and_then(|entry| entry.edit_tool),
                    "isDefault": model.id == cfg.default_model || model.id == DEFAULT_MODEL_ID,
                }));
            }
        }
        Ok(json!({ "models": models }))
    }

    pub async fn legacy_notifications_for_event(&self, envelope: EventEnvelope) -> Vec<Value> {
        match envelope.event {
            RoderEvent::SessionCreated(event) => {
                if let Some(thread) = self.legacy_thread_for_id(&event.thread_id, false).await {
                    vec![notification("thread/started", json!({ "thread": thread }))]
                } else {
                    Vec::new()
                }
            }
            RoderEvent::TurnStarted(event) => vec![
                notification(
                    "thread/status/changed",
                    json!({
                        "threadId": event.thread_id,
                        "status": LegacyThreadStatus::active(),
                    }),
                ),
                notification(
                    "turn/started",
                    json!({
                        "threadId": event.thread_id,
                        "turn": legacy_empty_turn(&event.turn_id, "inProgress"),
                    }),
                ),
                notification(
                    "item/started",
                    json!({
                        "threadId": event.thread_id,
                        "turnId": event.turn_id,
                        "item": {
                            "id": assistant_item_id(&event.turn_id),
                            "type": "agentMessage",
                            "text": "",
                        },
                    }),
                ),
            ],
            RoderEvent::InferenceEventReceived(event) => match event.event {
                InferenceEvent::MessageDelta(delta) => vec![notification(
                    "item/agentMessage/delta",
                    json!({
                        "threadId": event.thread_id,
                        "turnId": event.turn_id,
                        "itemId": assistant_item_id(&event.turn_id),
                        "delta": delta.text,
                        "phase": "final_answer",
                    }),
                )],
                InferenceEvent::ReasoningDelta(delta) => vec![notification(
                    "item/agentMessage/delta",
                    json!({
                        "threadId": event.thread_id,
                        "turnId": event.turn_id,
                        "itemId": assistant_item_id(&event.turn_id),
                        "delta": delta.text,
                        "phase": "reasoning",
                    }),
                )],
                InferenceEvent::ToolCallStarted(call) => vec![notification(
                    "item/completed",
                    json!({
                        "threadId": event.thread_id,
                        "turnId": event.turn_id,
                        "item": legacy_tool_status_item(&call.id, &call.name, "tool.started", ""),
                    }),
                )],
                InferenceEvent::ToolCallCompleted(call) => vec![notification(
                    "item/completed",
                    json!({
                        "threadId": event.thread_id,
                        "turnId": event.turn_id,
                        "item": legacy_inference_tool_call_item(&call),
                    }),
                )],
                InferenceEvent::Failed(failure) => vec![notification(
                    "item/completed",
                    json!({
                        "threadId": event.thread_id,
                        "turnId": event.turn_id,
                        "item": {
                            "id": format!("{}-error", event.turn_id),
                            "type": "error",
                            "text": failure.message,
                        },
                    }),
                )],
                _ => Vec::new(),
            },
            RoderEvent::ToolCallRequested(event) => vec![notification(
                "item/completed",
                json!({
                    "threadId": event.thread_id,
                    "turnId": event.turn_id,
                    "item": legacy_tool_status_item(&event.tool_id, &event.tool_name, "tool.requested", ""),
                }),
            )],
            RoderEvent::ToolCallStarted(event) => vec![notification(
                "item/completed",
                json!({
                    "threadId": event.thread_id,
                    "turnId": event.turn_id,
                    "item": legacy_tool_status_item(&event.tool_id, "tool", "tool.started", ""),
                }),
            )],
            RoderEvent::ToolCallCompleted(event) => vec![notification(
                "item/completed",
                json!({
                    "threadId": event.thread_id,
                    "turnId": event.turn_id,
                    "item": legacy_tool_status_item(&event.tool_id, "tool", "tool.completed", "completed"),
                }),
            )],
            RoderEvent::TurnCompleted(event) => {
                let mut out = Vec::new();
                if let Some(item) = self
                    .legacy_final_assistant_item(&event.thread_id, &event.turn_id)
                    .await
                {
                    out.push(notification(
                        "item/completed",
                        json!({
                            "threadId": event.thread_id,
                            "turnId": event.turn_id,
                            "item": item,
                        }),
                    ));
                }
                out.push(notification(
                    "thread/status/changed",
                    json!({
                        "threadId": event.thread_id,
                        "status": LegacyThreadStatus::idle(),
                    }),
                ));
                out.push(notification(
                    "turn/completed",
                    json!({
                        "threadId": event.thread_id,
                        "turn": legacy_empty_turn(&event.turn_id, "completed"),
                    }),
                ));
                out
            }
            RoderEvent::TurnFailed(event) => vec![
                notification(
                    "item/completed",
                    json!({
                        "threadId": event.thread_id,
                        "turnId": event.turn_id,
                        "item": {
                            "id": format!("{}-error", event.turn_id),
                            "type": "error",
                            "text": event.error,
                        },
                    }),
                ),
                notification(
                    "thread/status/changed",
                    json!({
                        "threadId": event.thread_id,
                        "status": LegacyThreadStatus::idle(),
                    }),
                ),
                notification(
                    "turn/completed",
                    json!({
                        "threadId": event.thread_id,
                        "turn": legacy_empty_turn(&event.turn_id, "failed"),
                    }),
                ),
            ],
            RoderEvent::TurnInterrupted(event) => vec![
                notification(
                    "thread/status/changed",
                    json!({
                        "threadId": event.thread_id,
                        "status": LegacyThreadStatus::idle(),
                    }),
                ),
                notification(
                    "turn/completed",
                    json!({
                        "threadId": event.thread_id,
                        "turn": legacy_empty_turn(&event.turn_id, "interrupted"),
                    }),
                ),
            ],
            _ => Vec::new(),
        }
    }

    async fn legacy_thread_for_id(
        &self,
        thread_id: &ThreadId,
        include_turns: bool,
    ) -> Option<Value> {
        let snapshot = self.runtime.load_session(thread_id).await.ok()??;
        let cfg = self.runtime.status().await;
        Some(legacy_thread_from_snapshot(snapshot, &cfg, include_turns))
    }

    async fn legacy_final_assistant_item(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
    ) -> Option<Value> {
        let snapshot = self.runtime.load_session(thread_id).await.ok()??;
        let turn = snapshot
            .turns
            .iter()
            .find(|candidate| &candidate.turn_id == turn_id)?;
        turn.items.iter().rev().find_map(|item| match item {
            ConversationItem::AssistantMessage(message) => Some(json!({
                "id": assistant_item_id(turn_id),
                "type": "agentMessage",
                "text": message.text,
                "phase": "final_answer",
            })),
            _ => None,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyThreadStartParams {
    cwd: Option<String>,
    model: Option<String>,
    model_provider: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyThreadListParams {
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyThreadReadParams {
    thread_id: String,
    #[serde(default)]
    include_turns: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyTurnStartParams {
    thread_id: String,
    prompt: Option<String>,
    input: Option<Vec<LegacyInputItem>>,
}

#[derive(Debug, Deserialize)]
struct LegacyInputItem {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyTurnInterruptParams {
    thread_id: String,
    turn_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyThreadStatus {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    active_flags: Vec<String>,
}

impl LegacyThreadStatus {
    fn idle() -> Self {
        Self {
            kind: "idle",
            active_flags: Vec::new(),
        }
    }

    fn active() -> Self {
        Self {
            kind: "active",
            active_flags: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyThread {
    id: String,
    session_id: String,
    preview: String,
    model_provider: String,
    created_at: i64,
    updated_at: i64,
    status: LegacyThreadStatus,
    cwd: String,
    source: &'static str,
    name: Option<String>,
    turns: Vec<LegacyTurn>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyTurn {
    id: String,
    items: Vec<Value>,
    items_view: &'static str,
    status: String,
    error: Option<LegacyTurnError>,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    duration_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
struct LegacyTurnError {
    message: String,
}

fn legacy_capabilities() -> Value {
    json!({
        "methods": [
            "initialize",
            "thread/start",
            "thread/list",
            "thread/read",
            "turn/start",
            "turn/steer",
            "turn/interrupt",
            "model/list",
        ],
        "turnInput": {
            "types": ["text", "local_file"],
            "localFileBinaryPolicy": "metadata",
        },
    })
}

fn legacy_thread_from_snapshot(
    snapshot: ThreadSnapshot,
    cfg: &roder_core::RuntimeConfig,
    include_turns: bool,
) -> Value {
    let metadata = snapshot.metadata.unwrap_or_else(|| SessionMetadata {
        thread_id: "unknown".to_string(),
        title: None,
        workspace: cfg.workspace.clone(),
        provider: Some(cfg.default_provider.clone()),
        model: Some(cfg.default_model.clone()),
        created_at: time::OffsetDateTime::UNIX_EPOCH,
        updated_at: time::OffsetDateTime::UNIX_EPOCH,
        message_count: 0,
    });
    let turns = if include_turns {
        snapshot
            .turns
            .iter()
            .map(|turn| legacy_turn_from_record(turn, &snapshot.events))
            .collect()
    } else {
        Vec::new()
    };
    legacy_thread_from_metadata(&metadata, cfg, turns)
}

fn legacy_thread_from_metadata(
    metadata: &SessionMetadata,
    cfg: &roder_core::RuntimeConfig,
    turns: Vec<LegacyTurn>,
) -> Value {
    let thread = LegacyThread {
        id: metadata.thread_id.clone(),
        session_id: metadata.thread_id.clone(),
        preview: metadata.title.clone().unwrap_or_default(),
        model_provider: metadata
            .provider
            .clone()
            .unwrap_or_else(|| cfg.default_provider.clone()),
        created_at: metadata.created_at.unix_timestamp(),
        updated_at: metadata.updated_at.unix_timestamp(),
        status: LegacyThreadStatus::idle(),
        cwd: metadata
            .workspace
            .clone()
            .or_else(|| cfg.workspace.clone())
            .unwrap_or_else(|| ".".to_string()),
        source: "appServer",
        name: metadata.title.clone(),
        turns,
    };
    serde_json::to_value(thread).unwrap()
}

fn legacy_turn_from_record(record: &TurnRecord, events: &[EventEnvelope]) -> LegacyTurn {
    let (status, error) = turn_status_and_error(&record.turn_id, events, record.completed_at);
    let started_at = Some(record.created_at.unix_timestamp());
    let completed_at = record.completed_at.map(|ts| ts.unix_timestamp());
    let duration_ms = completed_at
        .zip(started_at)
        .map(|(completed, started)| completed.saturating_sub(started).saturating_mul(1_000));
    LegacyTurn {
        id: record.turn_id.clone(),
        items: record
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| legacy_item_from_conversation(&record.turn_id, index, item))
            .collect(),
        items_view: "full",
        status,
        error,
        started_at,
        completed_at,
        duration_ms,
    }
}

fn turn_status_and_error(
    turn_id: &TurnId,
    events: &[EventEnvelope],
    completed_at: Option<time::OffsetDateTime>,
) -> (String, Option<LegacyTurnError>) {
    for envelope in events.iter().rev() {
        match &envelope.event {
            RoderEvent::TurnFailed(event) if &event.turn_id == turn_id => {
                return (
                    "failed".to_string(),
                    Some(LegacyTurnError {
                        message: event.error.clone(),
                    }),
                );
            }
            RoderEvent::TurnInterrupted(event) if &event.turn_id == turn_id => {
                return ("interrupted".to_string(), None);
            }
            RoderEvent::TurnCompleted(event) if &event.turn_id == turn_id => {
                return ("completed".to_string(), None);
            }
            _ => {}
        }
    }
    if completed_at.is_some() {
        ("completed".to_string(), None)
    } else {
        ("inProgress".to_string(), None)
    }
}

fn legacy_item_from_conversation(
    turn_id: &TurnId,
    index: usize,
    item: &ConversationItem,
) -> Option<Value> {
    match item {
        ConversationItem::UserMessage(message) => Some(json!({
            "id": format!("{turn_id}-user-{index}"),
            "type": "userMessage",
            "text": message.text,
        })),
        ConversationItem::AssistantMessage(message) => Some(json!({
            "id": assistant_item_id(turn_id),
            "type": "agentMessage",
            "text": message.text,
            "phase": "final_answer",
        })),
        ConversationItem::ReasoningSummary(summary) => Some(json!({
            "id": format!("{turn_id}-reasoning-{index}"),
            "type": "agentMessage",
            "text": summary.text,
            "phase": "reasoning",
        })),
        ConversationItem::ToolCall(call) => Some(legacy_tool_call_item(call)),
        ConversationItem::ToolResult(result) => Some(legacy_tool_result_item(result)),
        ConversationItem::Error(error) => Some(json!({
            "id": format!("{turn_id}-error-{index}"),
            "type": "error",
            "text": error.message,
        })),
        ConversationItem::FileChange(change) => Some(json!({
            "id": format!("{turn_id}-file-{index}"),
            "type": "tool.completed",
            "text": change.path,
            "payload": {
                "path": change.path,
                "changeType": change.change_type,
            },
        })),
        ConversationItem::ContextCompaction(_) | ConversationItem::ProviderMetadata(_) => None,
    }
}

fn legacy_empty_turn(turn_id: &TurnId, status: &str) -> Value {
    json!({
        "id": turn_id,
        "items": [],
        "itemsView": "full",
        "status": status,
    })
}

fn legacy_tool_call_item(call: &ToolCallRecord) -> Value {
    json!({
        "id": call.id,
        "type": "toolCall",
        "text": call.arguments,
        "toolName": call.name,
        "toolCallId": call.id,
        "payload": {
            "name": call.name,
            "arguments": call.arguments,
        },
    })
}

fn legacy_inference_tool_call_item(call: &roder_api::inference::ToolCallCompleted) -> Value {
    json!({
        "id": call.id,
        "type": "toolCall",
        "text": call.arguments,
        "toolName": call.name,
        "toolCallId": call.id,
        "payload": {
            "name": call.name,
            "arguments": call.arguments,
        },
    })
}

fn legacy_tool_result_item(result: &ToolResultRecord) -> Value {
    json!({
        "id": format!("{}-result", result.id),
        "type": if result.is_error { "tool.failed" } else { "tool.completed" },
        "text": result.result,
        "toolName": result.name,
        "toolCallId": result.id,
        "status": if result.is_error { "failed" } else { "completed" },
        "payload": {
            "output": result.result,
            "error": if result.is_error { Some(result.result.clone()) } else { None },
        },
    })
}

fn legacy_tool_status_item(tool_id: &str, tool_name: &str, item_type: &str, text: &str) -> Value {
    json!({
        "id": tool_id,
        "type": item_type,
        "text": text,
        "toolName": tool_name,
        "toolCallId": tool_id,
        "payload": {
            "name": tool_name,
            "output": text,
        },
    })
}

fn legacy_message_from_turn_params(params: &LegacyTurnStartParams) -> String {
    if let Some(input) = params.input.as_ref() {
        let mut text_parts = Vec::new();
        let mut file_paths = Vec::new();
        for item in input {
            match item.kind.as_str() {
                "text" => {
                    if let Some(text) = trim_nonempty(item.text.clone()) {
                        text_parts.push(text);
                    }
                }
                "local_file" | "file" => {
                    if let Some(path) = trim_nonempty(item.path.clone()) {
                        file_paths.push(path);
                    }
                }
                _ => {}
            }
        }
        if !file_paths.is_empty() {
            text_parts.push(format!("Attached local files:\n{}", file_paths.join("\n")));
        }
        return text_parts.join("\n\n");
    }
    params.prompt.clone().unwrap_or_default()
}

fn assistant_item_id(turn_id: &TurnId) -> String {
    format!("{turn_id}-assistant")
}

fn provider_for_model(model: &str) -> Option<&'static str> {
    lookup_model(model).map(|entry| entry.provider)
}

fn notification(method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

fn decode_optional<T>(params: Option<Value>) -> Result<T, JsonRpcError>
where
    T: serde::de::DeserializeOwned + Default,
{
    params
        .map(serde_json::from_value::<T>)
        .transpose()
        .map_err(invalid_params)
        .map(|params| params.unwrap_or_default())
}

fn decode_required<T>(params: Option<Value>) -> Result<T, JsonRpcError>
where
    T: serde::de::DeserializeOwned,
{
    let Some(params) = params else {
        return Err(invalid_params("Missing params"));
    };
    serde_json::from_value::<T>(params).map_err(invalid_params)
}

fn trim_nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {err}"),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32000,
        message: err.to_string(),
        data: None,
    }
}
