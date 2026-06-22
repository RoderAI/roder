//! Agent Client Protocol adapter for the Roder app-server.
//!
//! The app-server API is Roder-specific. This module exposes a small ACP v1
//! facade by translating ACP sessions and prompt turns onto the public
//! app-server JSON-RPC methods.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use agent_client_protocol_schema as acp;
use async_trait::async_trait;
use roder_protocol::{
    Item, JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, ThreadItemDelta,
    ThreadItemEvent, ThreadItemEventKind, ThreadItemStatus, ThreadResolveApprovalParams,
    ThreadStartParams, ThreadStartResult, TurnInputItem, TurnInterruptParams, TurnStartParams,
    TurnStartResult, WorkspaceCreateParams, WorkspaceCreateResult, WorkspaceRootInput,
};
use tokio::sync::Mutex;

use roder_app_server_core::{AppClient, AppNotificationReceiver};

#[derive(Clone)]
pub struct AcpAdapter<C> {
    client: C,
    sessions: Arc<Mutex<HashMap<String, AcpSession>>>,
}

#[derive(Debug, Clone)]
struct AcpSession {
    thread_id: String,
}

#[async_trait]
pub trait AcpClientPeer: Send + Sync {
    async fn send_notification(&self, notification: JsonRpcNotification) -> anyhow::Result<()>;

    async fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> anyhow::Result<acp::RequestPermissionResponse>;
}

impl<C> AcpAdapter<C>
where
    C: AppClient,
{
    pub fn new(client: C) -> Self {
        Self {
            client,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn handle_request<P>(
        &self,
        request: JsonRpcRequest,
        peer: &P,
    ) -> anyhow::Result<Option<JsonRpcResponse>>
    where
        P: AcpClientPeer,
    {
        let id = request.id.clone();
        if request.method == "session/cancel" {
            let result = self.handle_session_cancel(request.params).await;
            return Ok(id.map(|id| match result {
                Ok(()) => response_ok(Some(id), serde_json::json!({})),
                Err(error) => response_error(Some(id), error),
            }));
        }
        let result = match request.method.as_str() {
            "initialize" => self.handle_initialize(request.params).await,
            "session/new" => self.handle_session_new(request.params).await,
            "session/prompt" => self.handle_session_prompt(request.params, peer).await,
            _ => Err(method_not_found(request.method)),
        };

        Ok(id.map(|id| match result {
            Ok(value) => response_ok(Some(id), value),
            Err(error) => response_error(Some(id), error),
        }))
    }

    async fn handle_initialize(
        &self,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let request: acp::InitializeRequest = decode_params(params)?;
        let protocol_version = if request.protocol_version == acp::ProtocolVersion::V1 {
            acp::ProtocolVersion::V1
        } else {
            acp::ProtocolVersion::LATEST
        };
        let response = acp::InitializeResponse::new(protocol_version)
            .agent_capabilities(acp::AgentCapabilities::new())
            .agent_info(
                acp::Implementation::new("roder", env!("CARGO_PKG_VERSION")).title("Roder"),
            );
        serde_json::to_value(response).map_err(internal_error)
    }

    async fn handle_session_new(
        &self,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let request: acp::NewSessionRequest = decode_params(params)?;
        if !request.cwd.is_absolute() {
            return Err(invalid_params("session/new cwd must be absolute"));
        }
        if !request.additional_directories.is_empty() {
            return Err(invalid_params(
                "additionalDirectories are not advertised by Roder ACP",
            ));
        }
        if !request.mcp_servers.is_empty() {
            return Err(invalid_params("mcpServers are not advertised by Roder ACP"));
        }

        let workspace = self.create_workspace_for_cwd(&request.cwd).await?;
        let thread: ThreadStartResult = self
            .call(
                "thread/start",
                serde_json::to_value(ThreadStartParams {
                    selection: None,
                    model: None,
                    model_provider: None,
                    reasoning: None,
                    workspace_id: workspace.workspace.id.clone(),
                    root_id: Some(workspace.workspace.default_root_id.clone()),
                    cwd: Some(request.cwd.display().to_string()),
                    tool_allowlist: None,
                    developer_instructions: None,
                    external_tools: None,
                    runner: None,
                    ephemeral: false,
                })
                .map_err(internal_error)?,
            )
            .await?;
        self.sessions.lock().await.insert(
            thread.thread.id.clone(),
            AcpSession {
                thread_id: thread.thread.id.clone(),
            },
        );
        serde_json::to_value(acp::NewSessionResponse::new(thread.thread.id)).map_err(internal_error)
    }

    async fn handle_session_prompt<P>(
        &self,
        params: Option<serde_json::Value>,
        peer: &P,
    ) -> Result<serde_json::Value, JsonRpcError>
    where
        P: AcpClientPeer,
    {
        let request: acp::PromptRequest = decode_params(params)?;
        let session_id = request.session_id.to_string();
        let session = self
            .sessions
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| invalid_params(format!("unknown ACP session {session_id:?}")))?;
        let input = prompt_blocks_to_turn_input(request.prompt)?;
        let mut notifications = self.client.subscribe_notifications();
        let started: TurnStartResult = self
            .call(
                "turn/start",
                serde_json::to_value(TurnStartParams {
                    thread_id: session.thread_id.clone(),
                    input,
                    prompt: None,
                    model_provider: None,
                    model: None,
                    reasoning: None,
                    developer_context: None,
                    policy_mode: None,
                    task_ledger_required: false,
                })
                .map_err(internal_error)?,
            )
            .await?;

        loop {
            let notification = notifications.recv().await.map_err(internal_error)?;
            if !notification_matches_turn(&notification, &session.thread_id, &started.turn_id) {
                continue;
            }
            if let Some(update) = acp_session_update(&session_id, &notification) {
                peer.send_notification(update)
                    .await
                    .map_err(internal_error)?;
                continue;
            }
            if notification.method == "thread/approvalRequested" {
                self.handle_approval_request(&session_id, &notification, peer)
                    .await?;
                continue;
            }
            if notification.method == "turn/completed" {
                let stop_reason = turn_completed_stop_reason(&notification);
                return serde_json::to_value(acp::PromptResponse::new(stop_reason))
                    .map_err(internal_error);
            }
        }
    }

    async fn handle_session_cancel(
        &self,
        params: Option<serde_json::Value>,
    ) -> Result<(), JsonRpcError> {
        let request: acp::CancelNotification = decode_params(params)?;
        let session_id = request.session_id.to_string();
        let Some(session) = self.sessions.lock().await.get(&session_id).cloned() else {
            return Ok(());
        };
        let _: serde_json::Value = self
            .call(
                "turn/interrupt",
                serde_json::to_value(TurnInterruptParams {
                    thread_id: session.thread_id,
                    turn_id: None,
                })
                .map_err(internal_error)?,
            )
            .await?;
        Ok(())
    }

    async fn handle_approval_request<P>(
        &self,
        session_id: &str,
        notification: &JsonRpcNotification,
        peer: &P,
    ) -> Result<(), JsonRpcError>
    where
        P: AcpClientPeer,
    {
        notification_string(notification, "threadId")?;
        let approval_id = notification_string(notification, "approvalId")?;
        let tool_id = notification_string(notification, "toolId")?;
        let tool_name = notification_string(notification, "toolName")?;
        let reason = notification
            .params
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Roder needs permission to run this tool.");
        let content = vec![acp::ToolCallContent::from(reason.to_string())];
        let tool_call = acp::ToolCallUpdate::new(
            tool_id,
            acp::ToolCallUpdateFields::new()
                .title(tool_name.clone())
                .kind(tool_kind_for_name(&tool_name))
                .status(acp::ToolCallStatus::Pending)
                .content(content),
        );
        let response = peer
            .request_permission(acp::RequestPermissionRequest::new(
                session_id.to_string(),
                tool_call,
                vec![
                    acp::PermissionOption::new(
                        "allow_once",
                        "Allow once",
                        acp::PermissionOptionKind::AllowOnce,
                    ),
                    acp::PermissionOption::new(
                        "reject_once",
                        "Reject once",
                        acp::PermissionOptionKind::RejectOnce,
                    ),
                ],
            ))
            .await
            .map_err(internal_error)?;
        let approved = matches!(
            response.outcome,
            acp::RequestPermissionOutcome::Selected(ref selected)
                if selected.option_id.to_string() == "allow_once"
        );
        let _: serde_json::Value = self
            .call(
                "thread/resolve_approval",
                serde_json::to_value(ThreadResolveApprovalParams {
                    approval_id,
                    approved,
                })
                .map_err(internal_error)?,
            )
            .await?;
        Ok(())
    }

    async fn create_workspace_for_cwd(
        &self,
        cwd: &Path,
    ) -> Result<WorkspaceCreateResult, JsonRpcError> {
        self.call(
            "workspace/create",
            serde_json::to_value(WorkspaceCreateParams {
                name: None,
                roots: vec![WorkspaceRootInput {
                    path: cwd.display().to_string(),
                    name: None,
                }],
                default_root_path: Some(cwd.display().to_string()),
            })
            .map_err(internal_error)?,
        )
        .await
    }

    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, JsonRpcError> {
        let response = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(method)),
                method: method.to_string(),
                params: Some(params),
            })
            .await;
        if let Some(error) = response.error {
            return Err(error);
        }
        let result = response
            .result
            .ok_or_else(|| internal_error("missing result"))?;
        serde_json::from_value(result).map_err(internal_error)
    }
}

fn prompt_blocks_to_turn_input(
    prompt: Vec<acp::ContentBlock>,
) -> Result<Vec<TurnInputItem>, JsonRpcError> {
    let mut input = Vec::new();
    for block in prompt {
        match block {
            acp::ContentBlock::Text(text) => input.push(text_input(text.text)),
            acp::ContentBlock::ResourceLink(resource) => {
                input.push(text_input(resource_link_prompt_text(resource)));
            }
            acp::ContentBlock::Image(_)
            | acp::ContentBlock::Audio(_)
            | acp::ContentBlock::Resource(_) => {
                return Err(invalid_params(
                    "unsupported ACP prompt content type; Roder advertises only text and resource_link",
                ));
            }
            _ => {
                return Err(invalid_params(
                    "unsupported future ACP prompt content type; Roder advertises only text and resource_link",
                ));
            }
        }
    }
    Ok(input)
}

fn resource_link_prompt_text(resource: acp::ResourceLink) -> String {
    let mut text = format!("Resource link: {}\nURI: {}", resource.name, resource.uri);
    if let Some(title) = resource.title {
        text.push_str("\nTitle: ");
        text.push_str(&title);
    }
    if let Some(description) = resource.description {
        text.push_str("\nDescription: ");
        text.push_str(&description);
    }
    if let Some(mime_type) = resource.mime_type {
        text.push_str("\nMIME type: ");
        text.push_str(&mime_type);
    }
    text
}

fn text_input(text: String) -> TurnInputItem {
    TurnInputItem {
        kind: "text".to_string(),
        text: Some(text),
        path: None,
        image_url: None,
    }
}

fn acp_session_update(
    session_id: &str,
    notification: &JsonRpcNotification,
) -> Option<JsonRpcNotification> {
    let update = match notification.method.as_str() {
        "item/agentMessage/delta"
        | "item/reasoning/textDelta"
        | "item/reasoning/summaryTextDelta" => {
            let event: ThreadItemEvent =
                serde_json::from_value(notification.params.clone()).ok()?;
            session_update_for_item_delta(&event)?
        }
        "item/started" | "item/completed" => {
            let event: ThreadItemEvent =
                serde_json::from_value(notification.params.clone()).ok()?;
            session_update_for_item_lifecycle(&event)?
        }
        _ => return None,
    };
    let notification = acp::SessionNotification::new(session_id.to_string(), update);
    Some(JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: "session/update".to_string(),
        params: serde_json::to_value(notification).ok()?,
    })
}

fn session_update_for_item_delta(event: &ThreadItemEvent) -> Option<acp::SessionUpdate> {
    let ThreadItemEventKind::ItemDelta { item_id, delta } = &event.event else {
        return None;
    };
    match delta {
        ThreadItemDelta::AgentMessageText { delta, .. } => {
            Some(acp::SessionUpdate::AgentMessageChunk(
                acp::ContentChunk::new(text_content(delta.clone()))
                    .message_id(acp::MessageId::new(item_id.clone())),
            ))
        }
        ThreadItemDelta::ReasoningText { delta, .. }
        | ThreadItemDelta::ReasoningSummaryText { delta, .. } => {
            Some(acp::SessionUpdate::AgentThoughtChunk(
                acp::ContentChunk::new(text_content(delta.clone()))
                    .message_id(acp::MessageId::new(item_id.clone())),
            ))
        }
        ThreadItemDelta::ReasoningSummaryPartAdded { .. } => None,
    }
}

fn session_update_for_item_lifecycle(event: &ThreadItemEvent) -> Option<acp::SessionUpdate> {
    match &event.event {
        ThreadItemEventKind::ItemStarted { item } => tool_call_started(item),
        ThreadItemEventKind::ItemCompleted { item } => tool_call_completed(item),
        ThreadItemEventKind::ItemDelta { .. } => None,
    }
}

fn tool_call_started(item: &Item) -> Option<acp::SessionUpdate> {
    let Item::ToolExecution {
        tool_call_id,
        tool_name,
        status,
        input,
        ..
    } = item
    else {
        return None;
    };
    let mut call = acp::ToolCall::new(tool_call_id.clone(), tool_name.clone())
        .kind(tool_kind_for_name(tool_name))
        .status(tool_status(*status));
    if let Some(input) = input.clone() {
        call = call.raw_input(input);
    }
    Some(acp::SessionUpdate::ToolCall(call))
}

fn tool_call_completed(item: &Item) -> Option<acp::SessionUpdate> {
    let Item::ToolExecution {
        tool_call_id,
        tool_name,
        status,
        output,
        error,
        ..
    } = item
    else {
        return None;
    };
    let mut fields = acp::ToolCallUpdateFields::new()
        .title(tool_name.clone())
        .kind(tool_kind_for_name(tool_name))
        .status(tool_status(*status));
    let mut content = Vec::new();
    if let Some(output) = output {
        content.push(acp::ToolCallContent::from(output.clone()));
    }
    if let Some(error) = error {
        content.push(acp::ToolCallContent::from(error.clone()));
    }
    if !content.is_empty() {
        fields = fields.content(content);
    }
    Some(acp::SessionUpdate::ToolCallUpdate(
        acp::ToolCallUpdate::new(tool_call_id.clone(), fields),
    ))
}

fn text_content(text: String) -> acp::ContentBlock {
    acp::ContentBlock::Text(acp::TextContent::new(text))
}

fn tool_status(status: ThreadItemStatus) -> acp::ToolCallStatus {
    match status {
        ThreadItemStatus::InProgress => acp::ToolCallStatus::InProgress,
        ThreadItemStatus::Completed => acp::ToolCallStatus::Completed,
        ThreadItemStatus::Failed => acp::ToolCallStatus::Failed,
    }
}

fn tool_kind_for_name(name: &str) -> acp::ToolKind {
    let lower = name.to_ascii_lowercase();
    if lower.contains("read") || lower.contains("list") || lower.contains("cat") {
        acp::ToolKind::Read
    } else if lower.contains("write") || lower.contains("edit") || lower.contains("patch") {
        acp::ToolKind::Edit
    } else if lower.contains("delete") || lower.contains("remove") {
        acp::ToolKind::Delete
    } else if lower.contains("grep") || lower.contains("search") || lower.contains("find") {
        acp::ToolKind::Search
    } else if lower.contains("exec") || lower.contains("shell") || lower.contains("command") {
        acp::ToolKind::Execute
    } else {
        acp::ToolKind::Other
    }
}

fn notification_matches_turn(
    notification: &JsonRpcNotification,
    thread_id: &str,
    turn_id: &str,
) -> bool {
    let params = &notification.params;
    params
        .get("threadId")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|id| id == thread_id)
        && params
            .get("turnId")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                params
                    .get("turn")
                    .and_then(|turn| turn.get("id"))
                    .and_then(serde_json::Value::as_str)
            })
            .is_some_and(|id| id == turn_id)
}

fn turn_completed_stop_reason(notification: &JsonRpcNotification) -> acp::StopReason {
    let status = notification
        .params
        .get("turn")
        .and_then(|turn| turn.get("status"))
        .and_then(serde_json::Value::as_str);
    if status == Some("interrupted") {
        return acp::StopReason::Cancelled;
    }
    let finish_reason = notification
        .params
        .get("turn")
        .and_then(|turn| turn.get("finishReason"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    match finish_reason {
        "length" | "max_tokens" => acp::StopReason::MaxTokens,
        "max_turn_requests" => acp::StopReason::MaxTurnRequests,
        "refusal" => acp::StopReason::Refusal,
        "cancelled" | "canceled" => acp::StopReason::Cancelled,
        _ => acp::StopReason::EndTurn,
    }
}

fn notification_string(
    notification: &JsonRpcNotification,
    field: &str,
) -> Result<String, JsonRpcError> {
    notification
        .params
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| internal_error(format!("missing notification field {field}")))
}

fn decode_params<T: serde::de::DeserializeOwned>(
    params: Option<serde_json::Value>,
) -> Result<T, JsonRpcError> {
    serde_json::from_value(params.unwrap_or_else(|| serde_json::json!({}))).map_err(invalid_params)
}

pub fn response_ok(id: Option<serde_json::Value>, result: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(result),
        error: None,
    }
}

pub fn response_error(id: Option<serde_json::Value>, error: JsonRpcError) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(error),
    }
}

pub fn parse_error(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32700,
        message: format!("Parse error: {err}"),
        data: None,
    }
}

fn method_not_found(method: String) -> JsonRpcError {
    JsonRpcError {
        code: -32601,
        message: format!("Method not found: {method}"),
        data: None,
    }
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {err}"),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = err.to_string();
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}
