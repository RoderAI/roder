use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputImage {
    pub image_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserMessage {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<InputImage>,
}

impl UserMessage {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
        }
    }

    pub fn with_images(text: impl Into<String>, images: Vec<InputImage>) -> Self {
        Self {
            text: text.into(),
            images,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMessage {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReasoningSummary {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResultRecord {
    pub id: String,
    pub name: Option<String>,
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_payload: Option<serde_json::Value>,
    pub is_error: bool,
}

/// Reserved `display_payload` key carrying an image content block produced by
/// `view_image` and desktop observation tools. Providers forward the inline
/// image to the model and ACP clients receive an image content block. Kept out
/// of the scalar allow-list because it is a large data URL, not a display field.
pub const VIEW_IMAGE_DISPLAY_KEY: &str = "__view_image";

/// A bounded inline image carried by a tool result. Never fetch external URLs
/// while replaying a tool result or forwarding it to an ACP client.
pub fn tool_result_image(payload: Option<&Value>) -> Option<(&str, &str)> {
    use base64::Engine;
    let url = payload?
        .get(VIEW_IMAGE_DISPLAY_KEY)?
        .get("image_url")?
        .as_str()?;
    let (mime, data) = url.strip_prefix("data:")?.split_once(";base64,")?;
    if !matches!(
        mime,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    ) || data.is_empty()
        || data.len() > 12 * 1024 * 1024
    {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    Some((mime, data))
}

pub fn tool_display_payload(
    tool_name: Option<&str>,
    arguments: Option<&Value>,
    data: Option<&Value>,
) -> Option<Value> {
    let mut payload = Map::new();
    merge_display_fields(&mut payload, arguments);
    if tool_name.is_some_and(|name| name.starts_with("cua_"))
        && let Some(Value::Object(arguments)) = arguments
        && serde_json::to_vec(arguments).is_ok_and(|bytes| bytes.len() <= 32 * 1024)
    {
        payload.extend(arguments.clone());
    }
    merge_display_fields(&mut payload, data);
    if let Some(Value::Object(source)) = data
        && let Some(image) = source.get(VIEW_IMAGE_DISPLAY_KEY)
    {
        payload.insert(VIEW_IMAGE_DISPLAY_KEY.to_string(), image.clone());
    }
    (!payload.is_empty()).then_some(Value::Object(payload))
}

fn merge_display_fields(payload: &mut Map<String, Value>, source: Option<&Value>) {
    let Some(Value::Object(source)) = source else {
        return;
    };
    for key in [
        "path",
        "dir",
        "directory",
        "file",
        "action",
        "query",
        "url",
        "pattern",
        "regex",
        "glob",
        "command",
        "cmd",
        "shell_command",
        "name",
        "displayName",
        "skill",
        "shown",
        "total_lines",
        "next_offset",
        "truncated",
        "engine",
        "candidate_files",
        "verified_files",
        "elapsed_ms",
        "index_bytes",
        "index_build_time_ms",
    ] {
        let Some(value) = source.get(key).and_then(display_value) else {
            continue;
        };
        payload.insert(key.to_string(), value);
    }
}

fn display_value(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) if !text.is_empty() && text.len() <= 500 => Some(value.clone()),
        Value::Number(_) | Value::Bool(_) => Some(value.clone()),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChangeRecord {
    pub path: String,
    pub change_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextCompactionRecord {
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorRecord {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TranscriptItem {
    UserMessage(UserMessage),
    AssistantMessage(AssistantMessage),
    ReasoningSummary(ReasoningSummary),
    ToolCall(ToolCallRecord),
    ToolResult(ToolResultRecord),
    FileChange(FileChangeRecord),
    ContextCompaction(ContextCompactionRecord),
    Error(ErrorRecord),
    ProviderMetadata(serde_json::Value),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_display_payload_keeps_only_small_whitelisted_fields() {
        let payload = tool_display_payload(
            Some("write_file"),
            Some(&json!({
                "path": "src/lib.rs",
                "command": "cargo test",
                "content": "do not persist me",
                "query": "needle",
                "api_key": "secret"
            })),
            Some(&json!({
                "path": "src/main.rs",
                "shown": 4,
                "truncated": false,
                "engine": "indexed",
                "candidate_files": 2,
                "elapsed_ms": 5,
                "hunks": [{ "path": "src/main.rs" }]
            })),
        )
        .expect("display payload");

        assert_eq!(payload["path"], "src/main.rs");
        assert_eq!(payload["command"], "cargo test");
        assert_eq!(payload["query"], "needle");
        assert_eq!(payload["shown"], 4);
        assert_eq!(payload["truncated"], false);
        assert_eq!(payload["engine"], "indexed");
        assert_eq!(payload["candidate_files"], 2);
        assert_eq!(payload["elapsed_ms"], 5);
        assert!(payload.get("content").is_none());
        assert!(payload.get("api_key").is_none());
        assert!(payload.get("hunks").is_none());
    }
}
