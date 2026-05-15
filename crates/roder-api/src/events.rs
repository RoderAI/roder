use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::extension::{ExtensionId, InferenceEngineId};
use crate::inference::InferenceEvent;

pub type ThreadId = String;
pub type TurnId = String;
pub type EventId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventSource {
    Runtime,
    Core,
    Provider,
    Tool,
    AppServer,
    Tui,
    Extension,
    System,
}

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

impl RoderEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            RoderEvent::RuntimeStarted(_) => "runtime.started",
            RoderEvent::ExtensionRegistered(_) => "extension.registered",
            RoderEvent::SessionCreated(_) => "session.created",
            RoderEvent::SessionLoaded(_) => "session.loaded",
            RoderEvent::TurnStarted(_) => "turn.started",
            RoderEvent::ContextAssemblyStarted(_) => "context.assembly_started",
            RoderEvent::ContextBlockAdded(_) => "context.block_added",
            RoderEvent::ContextAssemblyCompleted(_) => "context.assembly_completed",
            RoderEvent::InferenceStarted(_) => "inference.started",
            RoderEvent::InferenceEventReceived(_) => "inference.event_received",
            RoderEvent::ToolCallRequested(_) => "tool.call_requested",
            RoderEvent::ApprovalRequested(_) => "approval.requested",
            RoderEvent::ApprovalResolved(_) => "approval.resolved",
            RoderEvent::ToolCallStarted(_) => "tool.call_started",
            RoderEvent::ToolCallCompleted(_) => "tool.call_completed",
            RoderEvent::FileChanged(_) => "file.changed",
            RoderEvent::TurnItemAppended(_) => "turn.item_appended",
            RoderEvent::TurnCompleted(_) => "turn.completed",
            RoderEvent::TurnFailed(_) => "turn.failed",
            RoderEvent::TurnInterrupted(_) => "turn.interrupted",
        }
    }

    pub fn source(&self) -> EventSource {
        match self {
            RoderEvent::InferenceEventReceived(_) | RoderEvent::InferenceStarted(_) => {
                EventSource::Provider
            }
            RoderEvent::ToolCallRequested(_)
            | RoderEvent::ToolCallStarted(_)
            | RoderEvent::ToolCallCompleted(_) => EventSource::Tool,
            RoderEvent::ExtensionRegistered(_) => EventSource::Extension,
            _ => EventSource::Core,
        }
    }

    pub fn thread_id(&self) -> Option<&ThreadId> {
        match self {
            RoderEvent::SessionCreated(e) => Some(&e.thread_id),
            RoderEvent::SessionLoaded(e) => Some(&e.thread_id),
            RoderEvent::TurnStarted(e) => Some(&e.thread_id),
            RoderEvent::ContextAssemblyStarted(e) => Some(&e.thread_id),
            RoderEvent::ContextBlockAdded(e) => Some(&e.thread_id),
            RoderEvent::ContextAssemblyCompleted(e) => Some(&e.thread_id),
            RoderEvent::InferenceStarted(e) => Some(&e.thread_id),
            RoderEvent::InferenceEventReceived(e) => Some(&e.thread_id),
            RoderEvent::ToolCallRequested(e) => Some(&e.thread_id),
            RoderEvent::ApprovalRequested(e) => Some(&e.thread_id),
            RoderEvent::ApprovalResolved(e) => Some(&e.thread_id),
            RoderEvent::ToolCallStarted(e) => Some(&e.thread_id),
            RoderEvent::ToolCallCompleted(e) => Some(&e.thread_id),
            RoderEvent::FileChanged(e) => Some(&e.thread_id),
            RoderEvent::TurnItemAppended(e) => Some(&e.thread_id),
            RoderEvent::TurnCompleted(e) => Some(&e.thread_id),
            RoderEvent::TurnFailed(e) => Some(&e.thread_id),
            RoderEvent::TurnInterrupted(e) => Some(&e.thread_id),
            RoderEvent::RuntimeStarted(_) | RoderEvent::ExtensionRegistered(_) => None,
        }
    }

    pub fn turn_id(&self) -> Option<&TurnId> {
        match self {
            RoderEvent::TurnStarted(e) => Some(&e.turn_id),
            RoderEvent::ContextAssemblyStarted(e) => Some(&e.turn_id),
            RoderEvent::ContextBlockAdded(e) => Some(&e.turn_id),
            RoderEvent::ContextAssemblyCompleted(e) => Some(&e.turn_id),
            RoderEvent::InferenceStarted(e) => Some(&e.turn_id),
            RoderEvent::InferenceEventReceived(e) => Some(&e.turn_id),
            RoderEvent::ToolCallRequested(e) => Some(&e.turn_id),
            RoderEvent::ApprovalRequested(e) => Some(&e.turn_id),
            RoderEvent::ApprovalResolved(e) => Some(&e.turn_id),
            RoderEvent::ToolCallStarted(e) => Some(&e.turn_id),
            RoderEvent::ToolCallCompleted(e) => Some(&e.turn_id),
            RoderEvent::FileChanged(e) => Some(&e.turn_id),
            RoderEvent::TurnItemAppended(e) => Some(&e.turn_id),
            RoderEvent::TurnCompleted(e) => Some(&e.turn_id),
            RoderEvent::TurnFailed(e) => Some(&e.turn_id),
            RoderEvent::TurnInterrupted(e) => Some(&e.turn_id),
            RoderEvent::RuntimeStarted(_)
            | RoderEvent::ExtensionRegistered(_)
            | RoderEvent::SessionCreated(_)
            | RoderEvent::SessionLoaded(_) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: EventId,
    pub seq: u64,
    pub timestamp: OffsetDateTime,
    pub source: EventSource,
    pub kind: String,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub event: RoderEvent,
}
