use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessage {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningSummary {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultRecord {
    pub id: String,
    pub result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChangeRecord {
    pub path: String,
    pub change_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCompactionRecord {
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversationItem {
    UserMessage(UserMessage),
    AssistantMessage(AssistantMessage),
    ReasoningSummary(ReasoningSummary),
    ToolCall(ToolCallRecord),
    ToolResult(ToolResultRecord),
    FileChange(FileChangeRecord),
    ContextCompaction(ContextCompactionRecord),
}

pub type TurnItem = ConversationItem;
