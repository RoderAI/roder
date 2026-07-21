use crate::error::{ClaudeSDKError, MessageParseError, Result};
use crate::types::{
    HookEventMessage, Message, MirrorErrorMessage, TaskNotificationMessage, TaskProgressMessage,
    TaskStartedMessage, TaskUpdatedMessage,
};

const KNOWN_MESSAGE_TYPES: &[&str] = &[
    "user",
    "assistant",
    "system",
    "result",
    "stream_event",
    "rate_limit_event",
];

pub fn parse_message_line(line: &str) -> Result<Option<Message>> {
    let value = serde_json::from_str::<serde_json::Value>(line)?;
    parse_message_value(value)
}

pub fn parse_message_value(value: serde_json::Value) -> Result<Option<Message>> {
    let message_type = value.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
        let data = value.as_object().cloned();
        let mut error = MessageParseError::new("Message missing 'type' field");
        if let Some(data) = data {
            error = error.with_data(data);
        }
        ClaudeSDKError::MessageParse(error)
    })?;

    if !KNOWN_MESSAGE_TYPES.contains(&message_type) {
        return Ok(None);
    }

    if message_type == "system" {
        return parse_system_message_value(value);
    }

    match serde_json::from_value::<Message>(value.clone()) {
        Ok(message) => Ok(Some(message)),
        Err(err) => Err(parse_error_with_payload(err, &value)),
    }
}

// Surface the offending payload alongside the serde error; a bare
// "invalid type: sequence, expected a map" is undebuggable without it.
fn parse_error_with_payload(err: serde_json::Error, value: &serde_json::Value) -> ClaudeSDKError {
    let payload = value.to_string();
    let payload = if payload.len() > 600 {
        let cut = payload
            .char_indices()
            .take_while(|(idx, _)| *idx <= 600)
            .last()
            .map(|(idx, ch)| idx + ch.len_utf8())
            .unwrap_or(payload.len());
        format!("{}...", &payload[..cut])
    } else {
        payload
    };
    let mut error = MessageParseError::new(format!(
        "Failed to parse CLI message: {err}; payload: {payload}"
    ));
    if let Some(data) = value.as_object() {
        error = error.with_data(data.clone());
    }
    ClaudeSDKError::MessageParse(error)
}

fn parse_system_message_value(value: serde_json::Value) -> Result<Option<Message>> {
    let subtype = value.get("subtype").and_then(|v| v.as_str());
    match subtype {
        Some("task_started") => parse_task_started(value)
            .map(Message::TaskStartedMsg)
            .map(Some),
        Some("task_progress") => parse_task_progress(value)
            .map(Message::TaskProgressMsg)
            .map(Some),
        Some("task_notification") => parse_task_notification(value)
            .map(Message::TaskNotificationMsg)
            .map(Some),
        Some("task_updated") => parse_task_updated(value)
            .map(Message::TaskUpdatedMsg)
            .map(Some),
        Some("hook_started" | "hook_response") => {
            parse_hook_event(value).map(Message::HookEventMsg).map(Some)
        }
        Some("mirror_error") => parse_mirror_error(value)
            .map(Message::MirrorErrorMsg)
            .map(Some),
        _ => serde_json::from_value::<Message>(value)
            .map(Some)
            .map_err(ClaudeSDKError::Serialization),
    }
}

fn parse_mirror_error(value: serde_json::Value) -> Result<MirrorErrorMessage> {
    let mut data = value.as_object().cloned().ok_or_else(|| {
        ClaudeSDKError::MessageParse(MessageParseError::new("System message must be an object"))
    })?;
    data.remove("type");
    let key = data.get("key").and_then(|value| value.as_object()).cloned();
    let error = data
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(MirrorErrorMessage { key, error, data })
}

fn parse_task_started(value: serde_json::Value) -> Result<TaskStartedMessage> {
    serde_json::from_value::<TaskStartedMessage>(strip_system_fields(value)?)
        .map_err(ClaudeSDKError::Serialization)
}

fn parse_task_progress(value: serde_json::Value) -> Result<TaskProgressMessage> {
    serde_json::from_value::<TaskProgressMessage>(strip_system_fields(value)?)
        .map_err(ClaudeSDKError::Serialization)
}

fn parse_task_notification(value: serde_json::Value) -> Result<TaskNotificationMessage> {
    serde_json::from_value::<TaskNotificationMessage>(strip_system_fields(value)?)
        .map_err(ClaudeSDKError::Serialization)
}

// Parsed defensively: a terminal task completion sometimes arrives only as a
// `task_updated` patch (no separate `task_notification`), and the patch may omit
// uuid/session_id. `status` is derived from `patch.status` (the CLI sets it on
// terminal transitions); an unrecognized status falls back to `None` while the
// full patch is preserved on `.patch` for callers that need more.
fn parse_task_updated(value: serde_json::Value) -> Result<TaskUpdatedMessage> {
    let data = value.as_object().ok_or_else(|| {
        ClaudeSDKError::MessageParse(MessageParseError::new("System message must be an object"))
    })?;
    let task_id = data
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let patch = data
        .get("patch")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let status = patch
        .get("status")
        .and_then(|v| serde_json::from_value::<crate::types::TaskUpdatedStatus>(v.clone()).ok());
    let session_id = data
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let uuid = data
        .get("uuid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok(TaskUpdatedMessage {
        task_id,
        patch,
        status,
        session_id,
        uuid,
    })
}

fn parse_hook_event(value: serde_json::Value) -> Result<HookEventMessage> {
    let mut data = value.as_object().cloned().ok_or_else(|| {
        ClaudeSDKError::MessageParse(MessageParseError::new("System message must be an object"))
    })?;
    let subtype = data
        .get("subtype")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let hook_event_name = data
        .get("hook_event")
        .or_else(|| data.get("hook_name"))
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let session_id = data
        .get("session_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let uuid = data
        .get("uuid")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    data.remove("type");
    Ok(HookEventMessage {
        subtype,
        hook_event_name,
        session_id,
        uuid,
        data,
    })
}

fn strip_system_fields(value: serde_json::Value) -> Result<serde_json::Value> {
    let mut data = value.as_object().cloned().ok_or_else(|| {
        ClaudeSDKError::MessageParse(MessageParseError::new("System message must be an object"))
    })?;
    data.remove("type");
    data.remove("subtype");
    Ok(serde_json::Value::Object(data))
}
