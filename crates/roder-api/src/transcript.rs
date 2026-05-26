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

pub fn tool_display_payload(
    _tool_name: Option<&str>,
    arguments: Option<&Value>,
    data: Option<&Value>,
) -> Option<Value> {
    let mut payload = Map::new();
    merge_display_fields(&mut payload, arguments);
    merge_display_fields(&mut payload, data);
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
