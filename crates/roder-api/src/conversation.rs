use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserMessage {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMessage {
    pub text: String,
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
    pub is_error: bool,
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
pub enum ConversationItem {
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

pub type TurnItem = ConversationItem;
