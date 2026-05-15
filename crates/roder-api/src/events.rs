use serde::{Serialize, Deserialize};
use time::OffsetDateTime;
use crate::extension::{ExtensionId, InferenceEngineId};
use crate::inference::InferenceEvent;

pub type ThreadId = String;
pub type TurnId = String;
pub type EventId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStarted {
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionRegistered {
    pub extension_id: ExtensionId,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCreated {
    pub thread_id: ThreadId,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLoaded {
    pub thread_id: ThreadId,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextAssemblyStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBlockAdded {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub block_type: String,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextAssemblyCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub engine_id: InferenceEngineId,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceEventReceived {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub event: InferenceEvent,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequested {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub tool_name: String,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequested {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub approval_id: String,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResolved {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub approval_id: String,
    pub approved: bool,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChanged {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub path: String,
    pub change_type: String,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnItemAppended {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub item_type: String,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnFailed {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub error: String,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnInterrupted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoderEvent {
    RuntimeStarted(RuntimeStarted),
    ExtensionRegistered(ExtensionRegistered),
    SessionCreated(SessionCreated),
    SessionLoaded(SessionLoaded),
    TurnStarted(TurnStarted),
    ContextAssemblyStarted(ContextAssemblyStarted),
    ContextBlockAdded(ContextBlockAdded),
    ContextAssemblyCompleted(ContextAssemblyCompleted),
    InferenceStarted(InferenceStarted),
    InferenceEventReceived(InferenceEventReceived),
    ToolCallRequested(ToolCallRequested),
    ApprovalRequested(ApprovalRequested),
    ApprovalResolved(ApprovalResolved),
    ToolCallStarted(ToolCallStarted),
    ToolCallCompleted(ToolCallCompleted),
    FileChanged(FileChanged),
    TurnItemAppended(TurnItemAppended),
    TurnCompleted(TurnCompleted),
    TurnFailed(TurnFailed),
    TurnInterrupted(TurnInterrupted),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: EventId,
    pub event: RoderEvent,
}
