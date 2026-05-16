use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::extension::{ExtensionId, InferenceEngineId};
use crate::inference::InferenceEvent;
use crate::subagents::SubagentExitReason;

pub use crate::policy_mode::{
    PolicyBypassActive, PolicyDecisionRecorded, PolicyExitPlanRequested, PolicyExitPlanResolved,
    PolicyModeChanged,
};
pub use crate::tasks::{TaskCancelled, TaskCompleted, TaskFailed, TaskOutput, TaskStarted};

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
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionRegistered {
    pub extension_id: ExtensionId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCreated {
    pub thread_id: ThreadId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLoaded {
    pub thread_id: ThreadId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextAssemblyStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBlockAdded {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub block_type: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextAssemblyCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub engine_id: InferenceEngineId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceEventReceived {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub event: InferenceEvent,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequested {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub tool_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequested {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub approval_id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResolved {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub approval_id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub approved: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentStarted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub parent_thread_id: ThreadId,
    pub parent_turn_id: TurnId,
    pub agent_type: String,
    pub description: String,
    pub model: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentMessage {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub parent_thread_id: ThreadId,
    pub parent_turn_id: TurnId,
    pub agent_type: String,
    pub text: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentToolCall {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub parent_thread_id: ThreadId,
    pub parent_turn_id: TurnId,
    pub agent_type: String,
    pub tool_id: String,
    pub tool_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub parent_thread_id: ThreadId,
    pub parent_turn_id: TurnId,
    pub agent_type: String,
    pub exit_reason: SubagentExitReason,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentFailed {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub parent_thread_id: ThreadId,
    pub parent_turn_id: TurnId,
    pub agent_type: String,
    pub error: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChanged {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub path: String,
    pub change_type: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChangePreviewReady {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub tool_name: String,
    pub path: String,
    pub change_type: String,
    pub before: Option<String>,
    pub after: String,
    pub supports_partial: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnItemAppended {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub item_type: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCompleted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnFailed {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub error: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnInterrupted {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnSteered {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub message: String,
    #[serde(with = "time::serde::rfc3339")]
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
    PolicyDecisionRecorded(PolicyDecisionRecorded),
    PolicyBypassActive(PolicyBypassActive),
    PolicyModeChanged(PolicyModeChanged),
    PolicyExitPlanRequested(PolicyExitPlanRequested),
    PolicyExitPlanResolved(PolicyExitPlanResolved),
    ToolCallStarted(ToolCallStarted),
    ToolCallCompleted(ToolCallCompleted),
    SubagentStarted(SubagentStarted),
    SubagentMessage(SubagentMessage),
    SubagentToolCall(SubagentToolCall),
    SubagentCompleted(SubagentCompleted),
    SubagentFailed(SubagentFailed),
    TaskStarted(TaskStarted),
    TaskOutput(TaskOutput),
    TaskCompleted(TaskCompleted),
    TaskFailed(TaskFailed),
    TaskCancelled(TaskCancelled),
    FileChangePreviewReady(FileChangePreviewReady),
    FileChanged(FileChanged),
    TurnItemAppended(TurnItemAppended),
    TurnCompleted(TurnCompleted),
    TurnFailed(TurnFailed),
    TurnInterrupted(TurnInterrupted),
    TurnSteered(TurnSteered),
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
            RoderEvent::PolicyDecisionRecorded(_) => "policy.decision",
            RoderEvent::PolicyBypassActive(_) => "policy.bypass_active",
            RoderEvent::PolicyModeChanged(_) => "policy.mode_changed",
            RoderEvent::PolicyExitPlanRequested(_) => "policy.exit_plan_requested",
            RoderEvent::PolicyExitPlanResolved(_) => "policy.exit_plan_resolved",
            RoderEvent::ToolCallStarted(_) => "tool.call_started",
            RoderEvent::ToolCallCompleted(_) => "tool.call_completed",
            RoderEvent::SubagentStarted(_) => "subagent.started",
            RoderEvent::SubagentMessage(_) => "subagent.message",
            RoderEvent::SubagentToolCall(_) => "subagent.tool_call",
            RoderEvent::SubagentCompleted(_) => "subagent.completed",
            RoderEvent::SubagentFailed(_) => "subagent.failed",
            RoderEvent::TaskStarted(_) => "task.started",
            RoderEvent::TaskOutput(_) => "task.output",
            RoderEvent::TaskCompleted(_) => "task.completed",
            RoderEvent::TaskFailed(_) => "task.failed",
            RoderEvent::TaskCancelled(_) => "task.cancelled",
            RoderEvent::FileChangePreviewReady(_) => "file.change_preview_ready",
            RoderEvent::FileChanged(_) => "file.changed",
            RoderEvent::TurnItemAppended(_) => "turn.item_appended",
            RoderEvent::TurnCompleted(_) => "turn.completed",
            RoderEvent::TurnFailed(_) => "turn.failed",
            RoderEvent::TurnInterrupted(_) => "turn.interrupted",
            RoderEvent::TurnSteered(_) => "turn.steered",
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
            RoderEvent::SubagentStarted(_)
            | RoderEvent::SubagentMessage(_)
            | RoderEvent::SubagentToolCall(_)
            | RoderEvent::SubagentCompleted(_)
            | RoderEvent::SubagentFailed(_)
            | RoderEvent::TaskStarted(_)
            | RoderEvent::TaskOutput(_)
            | RoderEvent::TaskCompleted(_)
            | RoderEvent::TaskFailed(_)
            | RoderEvent::TaskCancelled(_) => EventSource::Extension,
            RoderEvent::FileChangePreviewReady(_) => EventSource::Tool,
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
            RoderEvent::PolicyDecisionRecorded(e) => Some(&e.thread_id),
            RoderEvent::PolicyBypassActive(e) => Some(&e.thread_id),
            RoderEvent::PolicyModeChanged(e) => Some(&e.thread_id),
            RoderEvent::PolicyExitPlanRequested(e) => Some(&e.thread_id),
            RoderEvent::PolicyExitPlanResolved(e) => Some(&e.thread_id),
            RoderEvent::ToolCallStarted(e) => Some(&e.thread_id),
            RoderEvent::ToolCallCompleted(e) => Some(&e.thread_id),
            RoderEvent::SubagentStarted(e) => Some(&e.thread_id),
            RoderEvent::SubagentMessage(e) => Some(&e.thread_id),
            RoderEvent::SubagentToolCall(e) => Some(&e.thread_id),
            RoderEvent::SubagentCompleted(e) => Some(&e.thread_id),
            RoderEvent::SubagentFailed(e) => Some(&e.thread_id),
            RoderEvent::TaskStarted(e) => e.thread_id.as_ref(),
            RoderEvent::TaskOutput(e) => e.thread_id.as_ref(),
            RoderEvent::TaskCompleted(e) => e.thread_id.as_ref(),
            RoderEvent::TaskFailed(e) => e.thread_id.as_ref(),
            RoderEvent::TaskCancelled(e) => e.thread_id.as_ref(),
            RoderEvent::FileChangePreviewReady(e) => Some(&e.thread_id),
            RoderEvent::FileChanged(e) => Some(&e.thread_id),
            RoderEvent::TurnItemAppended(e) => Some(&e.thread_id),
            RoderEvent::TurnCompleted(e) => Some(&e.thread_id),
            RoderEvent::TurnFailed(e) => Some(&e.thread_id),
            RoderEvent::TurnInterrupted(e) => Some(&e.thread_id),
            RoderEvent::TurnSteered(e) => Some(&e.thread_id),
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
            RoderEvent::PolicyDecisionRecorded(e) => Some(&e.turn_id),
            RoderEvent::PolicyBypassActive(e) => Some(&e.turn_id),
            RoderEvent::PolicyModeChanged(e) => e.turn_id.as_ref(),
            RoderEvent::PolicyExitPlanRequested(e) => Some(&e.turn_id),
            RoderEvent::PolicyExitPlanResolved(e) => Some(&e.turn_id),
            RoderEvent::ToolCallStarted(e) => Some(&e.turn_id),
            RoderEvent::ToolCallCompleted(e) => Some(&e.turn_id),
            RoderEvent::SubagentStarted(e) => Some(&e.turn_id),
            RoderEvent::SubagentMessage(e) => Some(&e.turn_id),
            RoderEvent::SubagentToolCall(e) => Some(&e.turn_id),
            RoderEvent::SubagentCompleted(e) => Some(&e.turn_id),
            RoderEvent::SubagentFailed(e) => Some(&e.turn_id),
            RoderEvent::TaskStarted(e) => e.turn_id.as_ref(),
            RoderEvent::TaskOutput(e) => e.turn_id.as_ref(),
            RoderEvent::TaskCompleted(e) => e.turn_id.as_ref(),
            RoderEvent::TaskFailed(e) => e.turn_id.as_ref(),
            RoderEvent::TaskCancelled(e) => e.turn_id.as_ref(),
            RoderEvent::FileChangePreviewReady(e) => Some(&e.turn_id),
            RoderEvent::FileChanged(e) => Some(&e.turn_id),
            RoderEvent::TurnItemAppended(e) => Some(&e.turn_id),
            RoderEvent::TurnCompleted(e) => Some(&e.turn_id),
            RoderEvent::TurnFailed(e) => Some(&e.turn_id),
            RoderEvent::TurnInterrupted(e) => Some(&e.turn_id),
            RoderEvent::TurnSteered(e) => Some(&e.turn_id),
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
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    pub source: EventSource,
    pub kind: String,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub event: RoderEvent,
}

impl EventEnvelope {
    pub fn matches_filter(&self, filter: &EventFilter) -> bool {
        filter.matches(self)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventFilter {
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub source: Option<EventSource>,
    pub kinds: Vec<String>,
}

impl EventFilter {
    pub fn for_thread(thread_id: impl Into<ThreadId>) -> Self {
        Self {
            thread_id: Some(thread_id.into()),
            ..Self::default()
        }
    }

    pub fn for_turn(thread_id: impl Into<ThreadId>, turn_id: impl Into<TurnId>) -> Self {
        Self {
            thread_id: Some(thread_id.into()),
            turn_id: Some(turn_id.into()),
            ..Self::default()
        }
    }

    pub fn with_source(mut self, source: EventSource) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_kind(mut self, kind: impl Into<String>) -> Self {
        self.kinds.push(kind.into());
        self
    }

    pub fn matches(&self, envelope: &EventEnvelope) -> bool {
        if self
            .thread_id
            .as_ref()
            .is_some_and(|thread_id| envelope.thread_id.as_ref() != Some(thread_id))
        {
            return false;
        }
        if self
            .turn_id
            .as_ref()
            .is_some_and(|turn_id| envelope.turn_id.as_ref() != Some(turn_id))
        {
            return false;
        }
        if self
            .source
            .as_ref()
            .is_some_and(|source| &envelope.source != source)
        {
            return false;
        }
        if !self.kinds.is_empty() && !self.kinds.iter().any(|kind| kind == &envelope.kind) {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(thread_id: Option<&str>, turn_id: Option<&str>, kind: &str) -> EventEnvelope {
        EventEnvelope {
            event_id: "event-1".to_string(),
            seq: 1,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: EventSource::Core,
            kind: kind.to_string(),
            thread_id: thread_id.map(str::to_string),
            turn_id: turn_id.map(str::to_string),
            event: RoderEvent::TurnStarted(TurnStarted {
                thread_id: thread_id.unwrap_or("thread-a").to_string(),
                turn_id: turn_id.unwrap_or("turn-a").to_string(),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        }
    }

    #[test]
    fn event_filter_matches_thread_turn_source_and_kind() {
        let envelope = envelope(Some("thread-a"), Some("turn-a"), "turn.started");
        let filter = EventFilter::for_turn("thread-a", "turn-a")
            .with_source(EventSource::Core)
            .with_kind("turn.started");

        assert!(filter.matches(&envelope));
        assert!(envelope.matches_filter(&filter));
        assert!(!EventFilter::for_thread("thread-b").matches(&envelope));
        assert!(!EventFilter::for_turn("thread-a", "turn-b").matches(&envelope));
        assert!(
            !EventFilter::default()
                .with_source(EventSource::Provider)
                .matches(&envelope)
        );
        assert!(
            !EventFilter::default()
                .with_kind("turn.completed")
                .matches(&envelope)
        );
    }

    #[test]
    fn empty_event_filter_matches_everything() {
        assert!(EventFilter::default().matches(&envelope(None, None, "runtime.started")));
    }

    #[test]
    fn event_timestamps_serialize_as_rfc3339_strings() {
        let value =
            serde_json::to_value(envelope(Some("thread-a"), Some("turn-a"), "turn.started"))
                .unwrap();

        assert_eq!(value["timestamp"], "1970-01-01T00:00:00Z");
        assert_eq!(
            value["event"]["TurnStarted"]["timestamp"],
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn subagent_event_envelope_round_trips_parent_ids() {
        let event = RoderEvent::SubagentStarted(SubagentStarted {
            thread_id: "child-thread".to_string(),
            turn_id: "child-turn".to_string(),
            parent_thread_id: "parent-thread".to_string(),
            parent_turn_id: "parent-turn".to_string(),
            agent_type: "explore".to_string(),
            description: "Inspect repository".to_string(),
            model: Some("test-model".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });
        let envelope = EventEnvelope {
            event_id: "event-subagent-started".to_string(),
            seq: 7,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: event.source(),
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        };

        let serialized = serde_json::to_string(&envelope).unwrap();
        let round_trip: EventEnvelope = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip.kind, "subagent.started");
        assert_eq!(round_trip.source, EventSource::Extension);
        assert_eq!(round_trip.thread_id.as_deref(), Some("child-thread"));
        assert_eq!(round_trip.turn_id.as_deref(), Some("child-turn"));

        match round_trip.event {
            RoderEvent::SubagentStarted(started) => {
                assert_eq!(started.parent_thread_id, "parent-thread");
                assert_eq!(started.parent_turn_id, "parent-turn");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn task_events_round_trip_with_replay_ids() {
        let event = RoderEvent::TaskOutput(TaskOutput {
            task_id: "task-1".to_string(),
            stream: crate::tasks::TaskOutputStream::Stdout,
            chunk: "building\n".to_string(),
            dropped_bytes: 0,
            thread_id: Some("thread-a".to_string()),
            turn_id: Some("turn-a".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });
        let envelope = EventEnvelope {
            event_id: "event-task-output".to_string(),
            seq: 9,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: event.source(),
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        };

        let serialized = serde_json::to_string(&envelope).unwrap();
        let round_trip: EventEnvelope = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip.kind, "task.output");
        assert_eq!(round_trip.source, EventSource::Extension);
        assert_eq!(round_trip.thread_id.as_deref(), Some("thread-a"));
        assert_eq!(round_trip.turn_id.as_deref(), Some("turn-a"));
        match round_trip.event {
            RoderEvent::TaskOutput(output) => {
                assert_eq!(output.task_id, "task-1");
                assert_eq!(output.stream, crate::tasks::TaskOutputStream::Stdout);
                assert_eq!(output.chunk, "building\n");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn tool_call_completed_round_trips_error_status() {
        let event = RoderEvent::ToolCallCompleted(ToolCallCompleted {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            tool_id: "tool-a".to_string(),
            is_error: true,
            output: Some("tool failed".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });
        let envelope = EventEnvelope {
            event_id: "event-tool-completed".to_string(),
            seq: 10,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: event.source(),
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        };

        let serialized = serde_json::to_string(&envelope).unwrap();
        let round_trip: EventEnvelope = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip.kind, "tool.call_completed");
        match round_trip.event {
            RoderEvent::ToolCallCompleted(completed) => {
                assert_eq!(completed.tool_id, "tool-a");
                assert!(completed.is_error);
                assert_eq!(completed.output.as_deref(), Some("tool failed"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn file_change_preview_event_round_trips_public_metadata() {
        let event = RoderEvent::FileChangePreviewReady(FileChangePreviewReady {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            tool_id: "tool-a".to_string(),
            tool_name: "edit".to_string(),
            path: "src/lib.rs".to_string(),
            change_type: "modify".to_string(),
            before: Some("old\n".to_string()),
            after: "new\n".to_string(),
            supports_partial: false,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        });
        let envelope = EventEnvelope {
            event_id: "event-file-preview".to_string(),
            seq: 8,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: event.source(),
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        };

        let serialized = serde_json::to_string(&envelope).unwrap();
        let round_trip: EventEnvelope = serde_json::from_str(&serialized).unwrap();

        assert_eq!(round_trip.kind, "file.change_preview_ready");
        assert_eq!(round_trip.source, EventSource::Tool);
        assert_eq!(round_trip.thread_id.as_deref(), Some("thread-a"));
        assert_eq!(round_trip.turn_id.as_deref(), Some("turn-a"));
        match round_trip.event {
            RoderEvent::FileChangePreviewReady(preview) => {
                assert_eq!(preview.tool_id, "tool-a");
                assert_eq!(preview.tool_name, "edit");
                assert_eq!(preview.path, "src/lib.rs");
                assert_eq!(preview.change_type, "modify");
                assert_eq!(preview.before.as_deref(), Some("old\n"));
                assert_eq!(preview.after, "new\n");
                assert!(!preview.supports_partial);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
