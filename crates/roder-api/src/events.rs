use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::extension::{ExtensionId, InferenceEngineId};
use crate::inference::InferenceEvent;
use crate::subagents::SubagentExitReason;

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
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResolved {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub approval_id: String,
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
    SubagentStarted(SubagentStarted),
    SubagentMessage(SubagentMessage),
    SubagentToolCall(SubagentToolCall),
    SubagentCompleted(SubagentCompleted),
    SubagentFailed(SubagentFailed),
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
            RoderEvent::SubagentStarted(_) => "subagent.started",
            RoderEvent::SubagentMessage(_) => "subagent.message",
            RoderEvent::SubagentToolCall(_) => "subagent.tool_call",
            RoderEvent::SubagentCompleted(_) => "subagent.completed",
            RoderEvent::SubagentFailed(_) => "subagent.failed",
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
            RoderEvent::SubagentStarted(_)
            | RoderEvent::SubagentMessage(_)
            | RoderEvent::SubagentToolCall(_)
            | RoderEvent::SubagentCompleted(_)
            | RoderEvent::SubagentFailed(_) => EventSource::Extension,
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
            RoderEvent::SubagentStarted(e) => Some(&e.thread_id),
            RoderEvent::SubagentMessage(e) => Some(&e.thread_id),
            RoderEvent::SubagentToolCall(e) => Some(&e.thread_id),
            RoderEvent::SubagentCompleted(e) => Some(&e.thread_id),
            RoderEvent::SubagentFailed(e) => Some(&e.thread_id),
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
            RoderEvent::SubagentStarted(e) => Some(&e.turn_id),
            RoderEvent::SubagentMessage(e) => Some(&e.turn_id),
            RoderEvent::SubagentToolCall(e) => Some(&e.turn_id),
            RoderEvent::SubagentCompleted(e) => Some(&e.turn_id),
            RoderEvent::SubagentFailed(e) => Some(&e.turn_id),
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
}
