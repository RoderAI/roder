use agent_client_protocol_schema as acp;
use roder_protocol::{
    Item, JsonRpcError, JsonRpcNotification, ThreadItemDelta, ThreadItemEvent, ThreadItemEventKind,
    ThreadItemStatus, TurnInputItem,
};

use super::invalid_params;

pub(super) fn prompt_blocks_to_turn_input(
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

pub(crate) fn acp_session_update(
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
        "item/applyPatch/progress" => {
            let progress: roder_api::patch_progress::PatchProgress =
                serde_json::from_value(notification.params.clone()).ok()?;
            crate::patch_progress::acp_update(progress)
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
        input,
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
    if let Some(image) = crate::acp_media::tool_image(input.as_ref()) {
        content.push(image);
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

pub(super) fn tool_kind_for_name(name: &str) -> acp::ToolKind {
    if let Some(kind) = crate::acp_media::desktop_kind(name) {
        return kind;
    }
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
