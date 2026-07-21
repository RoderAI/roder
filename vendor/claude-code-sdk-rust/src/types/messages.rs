use serde::{Deserialize, Serialize};

/// Names of server-side tools the API executes on the model's behalf.
///
/// Mirrors the upstream Python `ServerToolName` literal. Deserializes
/// forward-compatibly: any unrecognized name is preserved in
/// [`ServerToolName::Other`] rather than failing to parse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum ServerToolName {
    Advisor,
    WebSearch,
    WebFetch,
    CodeExecution,
    BashCodeExecution,
    TextEditorCodeExecution,
    ToolSearchToolRegex,
    ToolSearchToolBm25,
    /// A server tool name not known at build time (forward-compatible).
    Other(String),
}

impl ServerToolName {
    /// The wire string for this server tool name.
    pub fn as_str(&self) -> &str {
        match self {
            ServerToolName::Advisor => "advisor",
            ServerToolName::WebSearch => "web_search",
            ServerToolName::WebFetch => "web_fetch",
            ServerToolName::CodeExecution => "code_execution",
            ServerToolName::BashCodeExecution => "bash_code_execution",
            ServerToolName::TextEditorCodeExecution => "text_editor_code_execution",
            ServerToolName::ToolSearchToolRegex => "tool_search_tool_regex",
            ServerToolName::ToolSearchToolBm25 => "tool_search_tool_bm25",
            ServerToolName::Other(name) => name.as_str(),
        }
    }
}

impl From<String> for ServerToolName {
    fn from(value: String) -> Self {
        match value.as_str() {
            "advisor" => ServerToolName::Advisor,
            "web_search" => ServerToolName::WebSearch,
            "web_fetch" => ServerToolName::WebFetch,
            "code_execution" => ServerToolName::CodeExecution,
            "bash_code_execution" => ServerToolName::BashCodeExecution,
            "text_editor_code_execution" => ServerToolName::TextEditorCodeExecution,
            "tool_search_tool_regex" => ServerToolName::ToolSearchToolRegex,
            "tool_search_tool_bm25" => ServerToolName::ToolSearchToolBm25,
            _ => ServerToolName::Other(value),
        }
    }
}

impl From<ServerToolName> for String {
    fn from(value: ServerToolName) -> Self {
        match value {
            ServerToolName::Other(name) => name,
            other => other.as_str().to_string(),
        }
    }
}

use super::{
    AssistantMessageErrorKind, RateLimitInfo, TaskNotificationStatus, TaskUpdatedStatus, TaskUsage,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Map<String, serde_json::Value>,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    ServerToolUse {
        id: String,
        name: ServerToolName,
        input: serde_json::Map<String, serde_json::Value>,
    },
    #[serde(rename = "advisor_tool_result")]
    ServerToolResult {
        tool_use_id: String,
        content: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    #[serde(rename = "user")]
    UserMsg {
        #[serde(rename = "message")]
        content: UserContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        uuid: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        // The CLI may serialize a tool result as either a structured object or
        // a bare string (e.g. a built-in tool's error text). Accept any JSON
        // value so message parsing never fails on a string payload.
        tool_use_result: Option<serde_json::Value>,
    },
    #[serde(rename = "assistant")]
    AssistantMsg {
        #[serde(rename = "message")]
        content: AssistantContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<AssistantMessageErrorKind>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<serde_json::Map<String, serde_json::Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        uuid: Option<String>,
    },
    #[serde(rename = "system")]
    SystemMsg {
        subtype: String,
        #[serde(flatten)]
        data: serde_json::Map<String, serde_json::Value>,
    },
    #[serde(skip_serializing, skip_deserializing)]
    TaskStartedMsg(TaskStartedMessage),
    #[serde(skip_serializing, skip_deserializing)]
    TaskProgressMsg(TaskProgressMessage),
    #[serde(skip_serializing, skip_deserializing)]
    TaskNotificationMsg(TaskNotificationMessage),
    #[serde(skip_serializing, skip_deserializing)]
    TaskUpdatedMsg(TaskUpdatedMessage),
    #[serde(skip_serializing, skip_deserializing)]
    HookEventMsg(HookEventMessage),
    #[serde(skip_serializing, skip_deserializing)]
    MirrorErrorMsg(MirrorErrorMessage),
    #[serde(rename = "result")]
    ResultMsg {
        subtype: String,
        duration_ms: i32,
        duration_api_ms: i32,
        is_error: bool,
        num_turns: i32,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        total_cost_usd: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<serde_json::Map<String, serde_json::Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        structured_output: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        deferred_tool_use: Option<DeferredToolUse>,
        #[serde(skip_serializing_if = "Option::is_none")]
        errors: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        api_error_status: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "modelUsage")]
        model_usage: Option<serde_json::Map<String, serde_json::Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        permission_denials: Option<Vec<serde_json::Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        uuid: Option<String>,
    },
    #[serde(rename = "stream_event")]
    StreamEventMsg {
        uuid: String,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        event: Option<serde_json::Map<String, serde_json::Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    #[serde(rename = "rate_limit_event")]
    RateLimitEventMsg {
        rate_limit_info: RateLimitInfo,
        uuid: String,
        session_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub content: Vec<ContentBlock>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "stop_reason")]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredToolUse {
    pub id: String,
    pub name: String,
    pub input: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStartedMessage {
    #[serde(alias = "task_id")]
    pub task_id: String,
    pub description: String,
    pub uuid: String,
    #[serde(alias = "session_id")]
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "tool_use_id")]
    pub tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "task_type")]
    pub task_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgressMessage {
    #[serde(alias = "task_id")]
    pub task_id: String,
    pub description: String,
    pub usage: TaskUsage,
    pub uuid: String,
    #[serde(alias = "session_id")]
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "tool_use_id")]
    pub tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "last_tool_name")]
    pub last_tool_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskNotificationMessage {
    #[serde(alias = "task_id")]
    pub task_id: String,
    pub status: TaskNotificationStatus,
    #[serde(alias = "output_file")]
    pub output_file: String,
    pub summary: String,
    pub uuid: String,
    #[serde(alias = "session_id")]
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "tool_use_id")]
    pub tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TaskUsage>,
}

/// System message emitted when a background task's state changes
/// (`system`/`task_updated`).
///
/// `patch` carries the changed fields (e.g. `status`, `end_time`); when
/// `patch.status` is terminal (see [`super::TERMINAL_TASK_STATUSES`]) the task
/// has finished. A background task's terminal state can arrive *only* as a
/// `TaskUpdatedMessage` with no accompanying `TaskNotificationMessage`, so
/// consumers tracking active task IDs should clear them on a terminal status
/// from either message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUpdatedMessage {
    #[serde(alias = "task_id")]
    pub task_id: String,
    pub patch: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskUpdatedStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "session_id")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEventMessage {
    pub subtype: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_event_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    pub data: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirrorErrorMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<serde_json::Map<String, serde_json::Value>>,
    pub error: String,
    pub data: serde_json::Map<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// User-supplied prompt input (text + images)
// ---------------------------------------------------------------------------

/// Source of an image supplied as part of a user prompt.
///
/// Serializes to the Claude Code stream-json image-source shape, matching the
/// Anthropic Messages API: a tagged `base64` source carries `media_type` plus
/// raw base64 `data`, while a `url` source references a remote image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

impl ImageSource {
    /// Parse a `data:<media_type>;base64,<data>` URL into a base64 image source.
    ///
    /// Returns `None` for inputs that are not base64-encoded `data:` URLs
    /// (e.g. remote `http(s)` URLs), which callers should route through
    /// [`ImageSource::Url`] or skip.
    pub fn from_data_url(url: &str) -> Option<Self> {
        let rest = url.strip_prefix("data:")?;
        let (meta, data) = rest.split_once(',')?;
        let meta = meta.strip_suffix(";base64")?;
        let media_type = if meta.is_empty() {
            "image/png".to_string()
        } else {
            meta.to_string()
        };
        Some(ImageSource::Base64 {
            media_type,
            data: data.to_string(),
        })
    }
}

/// A single block of user-supplied prompt content.
///
/// The Claude Code CLI accepts a user message whose `content` is either a
/// plain string or an array of these blocks; image blocks are the only way to
/// deliver real image input to the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputContentBlock {
    Text { text: String },
    Image { source: ImageSource },
}

impl InputContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        InputContentBlock::Text { text: text.into() }
    }

    pub fn image(source: ImageSource) -> Self {
        InputContentBlock::Image { source }
    }
}

/// User prompt content: either a plain string or a list of content blocks.
///
/// Passed to the streaming/query entrypoints. A plain string preserves the
/// historical text-only behaviour; the block form carries text and images
/// together so multimodal prompts reach the CLI as a real content array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserMessageInput {
    Text(String),
    Blocks(Vec<InputContentBlock>),
}

impl UserMessageInput {
    /// True when this input carries no text and no blocks.
    pub fn is_empty(&self) -> bool {
        match self {
            UserMessageInput::Text(text) => text.is_empty(),
            UserMessageInput::Blocks(blocks) => blocks.is_empty(),
        }
    }

    /// Render the value placed under the user message's `content` field: a JSON
    /// string for text input, or a JSON array of content blocks otherwise.
    pub fn to_content_value(&self) -> serde_json::Value {
        match self {
            UserMessageInput::Text(text) => serde_json::Value::String(text.clone()),
            UserMessageInput::Blocks(blocks) => serde_json::to_value(blocks)
                .unwrap_or_else(|_| serde_json::Value::Array(Vec::new())),
        }
    }
}

impl From<String> for UserMessageInput {
    fn from(value: String) -> Self {
        UserMessageInput::Text(value)
    }
}

impl From<&str> for UserMessageInput {
    fn from(value: &str) -> Self {
        UserMessageInput::Text(value.to_string())
    }
}

impl From<Vec<InputContentBlock>> for UserMessageInput {
    fn from(blocks: Vec<InputContentBlock>) -> Self {
        UserMessageInput::Blocks(blocks)
    }
}

#[cfg(test)]
mod input_content_tests {
    use super::*;

    #[test]
    fn parses_base64_data_url() {
        let src = ImageSource::from_data_url("data:image/png;base64,QUJD").unwrap();
        assert_eq!(
            src,
            ImageSource::Base64 {
                media_type: "image/png".to_string(),
                data: "QUJD".to_string(),
            }
        );
    }

    #[test]
    fn rejects_non_base64_url() {
        assert!(ImageSource::from_data_url("https://example.com/a.png").is_none());
        assert!(ImageSource::from_data_url("data:image/png,QUJD").is_none());
    }

    #[test]
    fn text_input_serializes_to_string_content() {
        let input: UserMessageInput = "hello".into();
        assert_eq!(input.to_content_value(), serde_json::json!("hello"));
    }

    #[test]
    fn block_input_serializes_to_content_array() {
        let input = UserMessageInput::Blocks(vec![
            InputContentBlock::text("look:"),
            InputContentBlock::image(ImageSource::Base64 {
                media_type: "image/png".to_string(),
                data: "QUJD".to_string(),
            }),
        ]);
        assert_eq!(
            input.to_content_value(),
            serde_json::json!([
                {"type": "text", "text": "look:"},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "QUJD"}}
            ])
        );
    }
}
