use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};
use time::OffsetDateTime;

use crate::events::{EventEnvelope, ThreadId, TurnId};
pub use crate::extension::{CheckpointStoreId, ThreadStoreId};
use crate::extension_state::ExtensionStateRecord;
use crate::inference::{InferenceEvent, TokenUsage, cache_hit_rate};
use crate::remote_runner::{RunnerDestination, RunnerSessionState};
use crate::transcript::{InputImage, TranscriptItem};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ThreadUsageMetadata {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub cached_prompt_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_rate: Option<f64>,
}

impl ThreadUsageMetadata {
    pub fn add_token_usage(&mut self, usage: &TokenUsage) {
        self.prompt_tokens = self
            .prompt_tokens
            .saturating_add(u64::from(usage.prompt_tokens));
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(u64::from(usage.completion_tokens));
        self.total_tokens = self
            .total_tokens
            .saturating_add(u64::from(usage.total_tokens));
        self.cached_prompt_tokens = self
            .cached_prompt_tokens
            .saturating_add(u64::from(usage.cached_prompt_tokens));
        self.cache_hit_rate = if self.prompt_tokens == 0 {
            None
        } else if self.prompt_tokens > u64::from(u32::MAX) {
            Some(
                (self.cached_prompt_tokens.min(self.prompt_tokens) as f64)
                    / (self.prompt_tokens as f64),
            )
        } else {
            cache_hit_rate(self.prompt_tokens as u32, self.cached_prompt_tokens as u32)
        };
    }

    pub fn is_empty(&self) -> bool {
        self.prompt_tokens == 0
            && self.completion_tokens == 0
            && self.total_tokens == 0
            && self.cached_prompt_tokens == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThreadMetadata {
    pub thread_id: ThreadId,
    pub title: Option<String>,
    #[serde(deserialize_with = "deserialize_thread_workspace")]
    pub workspace: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_destination: Option<RunnerDestination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_state: Option<RunnerSessionState>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub message_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ThreadUsageMetadata>,
}

pub fn validate_thread_workspace(workspace: &str) -> anyhow::Result<String> {
    let workspace = workspace.trim();
    anyhow::ensure!(!workspace.is_empty(), "thread workspace is required");
    anyhow::ensure!(
        std::path::Path::new(workspace).is_absolute(),
        "thread workspace must be an absolute path: {workspace}"
    );
    Ok(workspace.to_string())
}

fn deserialize_thread_workspace<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let workspace = String::deserialize(deserializer)?;
    validate_thread_workspace(&workspace).map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnRecord {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub items: Vec<TranscriptItem>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ThreadItemStatus {
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ThreadItem {
    UserMessage {
        id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<InputImage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ThreadItemStatus>,
    },
    AgentMessage {
        id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ThreadItemStatus>,
    },
    Reasoning {
        id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        summary: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        content: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ThreadItemStatus>,
    },
    ToolExecution {
        id: String,
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        status: ThreadItemStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Compaction {
        id: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ThreadItemStatus>,
    },
    Error {
        id: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ThreadItemStatus>,
    },
    Raw {
        id: String,
        payload: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ThreadItemStatus>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThreadItemTurnRecord {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub items: Vec<ThreadItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ThreadItemDelta {
    AgentMessageText {
        delta: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
    },
    ReasoningText {
        delta: String,
        #[serde(rename = "contentIndex")]
        content_index: usize,
    },
    ReasoningSummaryPartAdded {
        #[serde(rename = "summaryIndex")]
        summary_index: usize,
    },
    ReasoningSummaryText {
        delta: String,
        #[serde(rename = "summaryIndex")]
        summary_index: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ThreadItemEventKind {
    ItemStarted {
        item: ThreadItem,
    },
    ItemDelta {
        #[serde(rename = "itemId")]
        item_id: String,
        delta: ThreadItemDelta,
    },
    ItemCompleted {
        item: ThreadItem,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadItemEvent {
    pub seq: u64,
    #[serde(rename = "eventId")]
    pub event_id: String,
    #[serde(rename = "threadId")]
    pub thread_id: ThreadId,
    #[serde(rename = "turnId")]
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    pub event: ThreadItemEventKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadSnapshot {
    pub metadata: Option<ThreadMetadata>,
    pub events: Vec<EventEnvelope>,
    pub turns: Vec<TurnRecord>,
    #[serde(default)]
    pub item_events: Vec<ThreadItemEvent>,
    pub extension_states: Vec<ExtensionStateRecord>,
}

pub fn project_thread_item_events(events: &[ThreadItemEvent]) -> Vec<ThreadItemTurnRecord> {
    let mut turns = Vec::<ThreadItemTurnRecord>::new();
    for event in events {
        let turn_index = turns
            .iter()
            .position(|turn| turn.turn_id == event.turn_id)
            .unwrap_or_else(|| {
                turns.push(ThreadItemTurnRecord {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    created_at: event.timestamp,
                    items: Vec::new(),
                });
                turns.len() - 1
            });
        apply_thread_item_event(&mut turns[turn_index].items, &event.event);
    }
    turns
}

fn apply_thread_item_event(items: &mut Vec<ThreadItem>, event: &ThreadItemEventKind) {
    match event {
        ThreadItemEventKind::ItemStarted { item } => {
            if !upsert_item_if_missing(items, item.clone()) {
                merge_started_item(items, item);
            }
        }
        ThreadItemEventKind::ItemDelta { item_id, delta } => {
            let index = item_index(items, item_id).unwrap_or_else(|| {
                items.push(item_for_delta(item_id, delta));
                items.len() - 1
            });
            apply_thread_item_delta(&mut items[index], delta);
        }
        ThreadItemEventKind::ItemCompleted { item } => {
            if let Some(index) = item_index(items, item.id()) {
                merge_completed_item(&mut items[index], item.clone());
            } else {
                items.push(item.clone());
            }
        }
    }
}

fn upsert_item_if_missing(items: &mut Vec<ThreadItem>, item: ThreadItem) -> bool {
    if item_index(items, item.id()).is_some() {
        return false;
    }
    items.push(item);
    true
}

fn merge_started_item(items: &mut [ThreadItem], incoming: &ThreadItem) {
    if let Some(index) = item_index(items, incoming.id()) {
        merge_completed_item(&mut items[index], incoming.clone());
    }
}

fn item_index(items: &[ThreadItem], item_id: &str) -> Option<usize> {
    items.iter().position(|item| item.id() == item_id)
}

fn item_for_delta(item_id: &str, delta: &ThreadItemDelta) -> ThreadItem {
    match delta {
        ThreadItemDelta::AgentMessageText { phase, .. } => ThreadItem::AgentMessage {
            id: item_id.to_string(),
            text: String::new(),
            phase: phase.clone(),
            status: Some(ThreadItemStatus::InProgress),
        },
        ThreadItemDelta::ReasoningText { content_index, .. } => ThreadItem::Reasoning {
            id: item_id.to_string(),
            summary: Vec::new(),
            content: vec![String::new(); content_index + 1],
            status: Some(ThreadItemStatus::InProgress),
        },
        ThreadItemDelta::ReasoningSummaryPartAdded { summary_index }
        | ThreadItemDelta::ReasoningSummaryText { summary_index, .. } => ThreadItem::Reasoning {
            id: item_id.to_string(),
            summary: vec![String::new(); summary_index + 1],
            content: Vec::new(),
            status: Some(ThreadItemStatus::InProgress),
        },
    }
}

fn apply_thread_item_delta(item: &mut ThreadItem, delta: &ThreadItemDelta) {
    match (item, delta) {
        (
            ThreadItem::AgentMessage {
                text,
                phase,
                status,
                ..
            },
            ThreadItemDelta::AgentMessageText {
                delta,
                phase: delta_phase,
            },
        ) => {
            text.push_str(delta);
            if phase.is_none() {
                *phase = delta_phase.clone();
            }
            *status = Some(ThreadItemStatus::InProgress);
        }
        (
            ThreadItem::Reasoning {
                content, status, ..
            },
            ThreadItemDelta::ReasoningText {
                delta,
                content_index,
            },
        ) => {
            ensure_vec_slot(content, *content_index);
            content[*content_index].push_str(delta);
            *status = Some(ThreadItemStatus::InProgress);
        }
        (
            ThreadItem::Reasoning {
                summary, status, ..
            },
            ThreadItemDelta::ReasoningSummaryPartAdded { summary_index },
        ) => {
            ensure_vec_slot(summary, *summary_index);
            *status = Some(ThreadItemStatus::InProgress);
        }
        (
            ThreadItem::Reasoning {
                summary, status, ..
            },
            ThreadItemDelta::ReasoningSummaryText {
                delta,
                summary_index,
            },
        ) => {
            ensure_vec_slot(summary, *summary_index);
            summary[*summary_index].push_str(delta);
            *status = Some(ThreadItemStatus::InProgress);
        }
        (item, delta) => {
            *item = item_for_delta(item.id(), delta);
            apply_thread_item_delta(item, delta);
        }
    }
}

fn ensure_vec_slot(values: &mut Vec<String>, index: usize) {
    while values.len() <= index {
        values.push(String::new());
    }
}

fn merge_completed_item(existing: &mut ThreadItem, incoming: ThreadItem) {
    match (&mut *existing, incoming) {
        (
            ThreadItem::Reasoning {
                summary,
                content,
                status,
                ..
            },
            ThreadItem::Reasoning {
                summary: incoming_summary,
                content: incoming_content,
                status: incoming_status,
                ..
            },
        ) => {
            if !incoming_summary.is_empty() {
                *summary = incoming_summary;
            }
            if !incoming_content.is_empty() {
                *content = incoming_content;
            }
            *status = incoming_status.or(Some(ThreadItemStatus::Completed));
        }
        (
            ThreadItem::ToolExecution {
                status,
                input,
                output,
                error,
                ..
            },
            ThreadItem::ToolExecution {
                status: incoming_status,
                input: incoming_input,
                output: incoming_output,
                error: incoming_error,
                ..
            },
        ) => {
            *status = incoming_status;
            if incoming_input.is_some() {
                *input = incoming_input;
            }
            if incoming_output.is_some() {
                *output = incoming_output;
            }
            if incoming_error.is_some() {
                *error = incoming_error;
            }
        }
        (slot, incoming) => *slot = incoming,
    }
}

impl ThreadItem {
    pub fn id(&self) -> &str {
        match self {
            ThreadItem::UserMessage { id, .. }
            | ThreadItem::AgentMessage { id, .. }
            | ThreadItem::Reasoning { id, .. }
            | ThreadItem::ToolExecution { id, .. }
            | ThreadItem::Compaction { id, .. }
            | ThreadItem::Error { id, .. }
            | ThreadItem::Raw { id, .. } => id,
        }
    }
}

pub fn agent_message_item_id(turn_id: &str, phase: Option<&str>) -> String {
    format!("{}-agent-{}", turn_id, phase.unwrap_or("final_answer"))
}

pub fn thread_item_event_kind_item_id(event: &ThreadItemEventKind) -> &str {
    match event {
        ThreadItemEventKind::ItemStarted { item } => item.id(),
        ThreadItemEventKind::ItemDelta { item_id, .. } => item_id,
        ThreadItemEventKind::ItemCompleted { item } => item.id(),
    }
}

pub fn public_item_id_for_inference_event(turn_id: &str, event: &InferenceEvent) -> Option<String> {
    match event {
        InferenceEvent::MessageDelta(delta) => {
            Some(agent_message_item_id(turn_id, delta.phase.as_deref()))
        }
        InferenceEvent::ReasoningDelta(_) => {
            Some(agent_message_item_id(turn_id, Some("reasoning")))
        }
        InferenceEvent::HostedToolCallStarted(call) => Some(call.id.clone()),
        InferenceEvent::HostedToolCallCompleted(call) => Some(call.id.clone()),
        InferenceEvent::ToolCallStarted(call) => Some(call.id.clone()),
        InferenceEvent::ToolCallCompleted(call) => Some(call.id.clone()),
        _ => None,
    }
}

pub fn public_item_event_kinds_from_inference_event(
    turn_id: &str,
    event: &InferenceEvent,
    item_exists: bool,
) -> Vec<ThreadItemEventKind> {
    match event {
        InferenceEvent::MessageDelta(delta) => {
            let item_id = agent_message_item_id(turn_id, delta.phase.as_deref());
            let mut events = Vec::new();
            if !item_exists {
                events.push(ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::AgentMessage {
                        id: item_id.clone(),
                        text: String::new(),
                        phase: delta.phase.clone(),
                        status: Some(ThreadItemStatus::InProgress),
                    },
                });
            }
            events.push(ThreadItemEventKind::ItemDelta {
                item_id,
                delta: ThreadItemDelta::AgentMessageText {
                    delta: delta.text.clone(),
                    phase: delta.phase.clone(),
                },
            });
            events
        }
        InferenceEvent::ReasoningDelta(delta) => {
            let item_id = agent_message_item_id(turn_id, Some("reasoning"));
            let mut events = Vec::new();
            if !item_exists {
                events.push(ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::Reasoning {
                        id: item_id.clone(),
                        summary: Vec::new(),
                        content: vec![String::new()],
                        status: Some(ThreadItemStatus::InProgress),
                    },
                });
            }
            events.push(ThreadItemEventKind::ItemDelta {
                item_id,
                delta: ThreadItemDelta::ReasoningText {
                    delta: delta.text.clone(),
                    content_index: 0,
                },
            });
            events
        }
        InferenceEvent::HostedToolCallStarted(call) => vec![ThreadItemEventKind::ItemStarted {
            item: ThreadItem::ToolExecution {
                id: call.id.clone(),
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                status: ThreadItemStatus::InProgress,
                input: None,
                output: None,
                error: None,
            },
        }],
        InferenceEvent::HostedToolCallCompleted(call) => vec![ThreadItemEventKind::ItemCompleted {
            item: ThreadItem::ToolExecution {
                id: call.id.clone(),
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                status: ThreadItemStatus::Completed,
                input: serde_json::from_str(&call.arguments).ok(),
                output: None,
                error: None,
            },
        }],
        InferenceEvent::ToolCallStarted(call) => vec![ThreadItemEventKind::ItemStarted {
            item: ThreadItem::ToolExecution {
                id: call.id.clone(),
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                status: ThreadItemStatus::InProgress,
                input: None,
                output: None,
                error: None,
            },
        }],
        InferenceEvent::ToolCallCompleted(call) => vec![ThreadItemEventKind::ItemStarted {
            item: ThreadItem::ToolExecution {
                id: call.id.clone(),
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                status: ThreadItemStatus::InProgress,
                input: serde_json::from_str(&call.arguments).ok(),
                output: None,
                error: None,
            },
        }],
        _ => Vec::new(),
    }
}

pub fn public_thread_item_from_transcript_item(
    turn_id: &TurnId,
    index: usize,
    item: &TranscriptItem,
) -> ThreadItem {
    match item {
        TranscriptItem::UserMessage(message) => ThreadItem::UserMessage {
            id: format!("{turn_id}-user"),
            text: message.text.clone(),
            images: message.images.clone(),
            status: Some(ThreadItemStatus::Completed),
        },
        TranscriptItem::AssistantMessage(message) => ThreadItem::AgentMessage {
            id: agent_message_item_id(turn_id, message.phase.as_deref()),
            text: message.text.clone(),
            phase: message.phase.clone(),
            status: Some(ThreadItemStatus::Completed),
        },
        TranscriptItem::ReasoningSummary(summary) => ThreadItem::Reasoning {
            id: agent_message_item_id(turn_id, Some("reasoning")),
            summary: Vec::new(),
            content: vec![summary.text.clone()],
            status: Some(ThreadItemStatus::Completed),
        },
        TranscriptItem::ToolCall(call) => ThreadItem::ToolExecution {
            id: call.id.clone(),
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            status: ThreadItemStatus::InProgress,
            input: serde_json::from_str(&call.arguments).ok(),
            output: None,
            error: None,
        },
        TranscriptItem::ToolResult(result) => ThreadItem::ToolExecution {
            id: result.id.clone(),
            tool_call_id: result.id.clone(),
            tool_name: result.name.clone().unwrap_or_else(|| "tool".to_string()),
            status: if result.is_error {
                ThreadItemStatus::Failed
            } else {
                ThreadItemStatus::Completed
            },
            input: result.display_payload.clone(),
            output: (!result.is_error).then(|| result.result.clone()),
            error: result.is_error.then(|| result.result.clone()),
        },
        TranscriptItem::ContextCompaction(compaction) => ThreadItem::Compaction {
            id: format!("{turn_id}-compaction-{index}"),
            summary: compaction.summary.clone(),
            status: Some(ThreadItemStatus::Completed),
        },
        TranscriptItem::Error(error) => ThreadItem::Error {
            id: format!("{turn_id}-error-{index}"),
            message: error.message.clone(),
            status: Some(ThreadItemStatus::Failed),
        },
        other => ThreadItem::Raw {
            id: format!("{turn_id}-item-{index}"),
            payload: serde_json::to_value(other).unwrap_or(serde_json::Value::Null),
            status: Some(ThreadItemStatus::Completed),
        },
    }
}

pub fn public_item_event_kind_from_transcript_item(
    turn_id: &TurnId,
    index: usize,
    item: &TranscriptItem,
) -> ThreadItemEventKind {
    let item = public_thread_item_from_transcript_item(turn_id, index, item);
    match item {
        ThreadItem::ToolExecution {
            status: ThreadItemStatus::InProgress,
            ..
        } => ThreadItemEventKind::ItemStarted { item },
        _ => ThreadItemEventKind::ItemCompleted { item },
    }
}

pub fn public_item_event_from_transcript_item(
    thread_id: &ThreadId,
    turn_id: &TurnId,
    seq: u64,
    timestamp: OffsetDateTime,
    index: usize,
    item: &TranscriptItem,
) -> ThreadItemEvent {
    ThreadItemEvent {
        seq,
        event_id: format!("{turn_id}-item-event-{seq}"),
        thread_id: thread_id.clone(),
        turn_id: turn_id.clone(),
        timestamp,
        event: public_item_event_kind_from_transcript_item(turn_id, index, item),
    }
}

#[async_trait::async_trait]
pub trait ThreadStore: Send + Sync {
    fn id(&self) -> ThreadStoreId;

    fn local_thread_root(&self) -> Option<PathBuf> {
        None
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata>;
    async fn update_thread_metadata(
        &self,
        metadata: ThreadMetadata,
    ) -> anyhow::Result<ThreadMetadata> {
        Ok(metadata)
    }
    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>>;
    async fn load_thread(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>>;
    async fn archive_thread(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        let _ = thread_id;
        anyhow::bail!("thread store {} does not support archive", self.id())
    }
    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()>;
    async fn append_turn_item(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        item: &TranscriptItem,
    ) -> anyhow::Result<()>;
    async fn append_item_event(
        &self,
        thread_id: &ThreadId,
        item_event: &ThreadItemEvent,
    ) -> anyhow::Result<()> {
        let _ = (thread_id, item_event);
        Ok(())
    }
    async fn append_extension_state(
        &self,
        thread_id: &ThreadId,
        record: &ExtensionStateRecord,
    ) -> anyhow::Result<()> {
        let _ = (thread_id, record);
        anyhow::bail!(
            "thread store {} does not support extension state",
            self.id()
        )
    }
}

pub trait ThreadStoreFactory: Send + Sync + 'static {
    fn id(&self) -> ThreadStoreId;
    fn create(&self) -> Arc<dyn ThreadStore>;
}

#[async_trait::async_trait]
pub trait CheckpointStore: Send + Sync {
    fn id(&self) -> CheckpointStoreId;
    async fn save_snapshot(&self, snapshot: ThreadSnapshot) -> anyhow::Result<()>;
    async fn load_snapshot(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>>;
}

pub trait CheckpointStoreFactory: Send + Sync + 'static {
    fn id(&self) -> CheckpointStoreId;
    fn create(&self) -> Arc<dyn CheckpointStore>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{InferenceEvent, MessageDelta, ReasoningDelta};

    #[test]
    fn thread_metadata_timestamps_serialize_as_rfc3339_strings() {
        let value = serde_json::to_value(ThreadMetadata {
            thread_id: "thread-a".to_string(),
            title: None,
            workspace: "/workspace".to_string(),
            provider: None,
            model: None,
            runner_destination: None,
            runner_state: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            message_count: 0,
            usage: None,
        })
        .unwrap();

        assert_eq!(value["created_at"], "1970-01-01T00:00:00Z");
        assert_eq!(value["updated_at"], "1970-01-01T00:00:00Z");
        assert_eq!(value["workspace"], "/workspace");
    }

    #[test]
    fn thread_metadata_requires_workspace_when_deserializing() {
        let value = serde_json::json!({
            "thread_id": "thread-a",
            "title": null,
            "provider": null,
            "model": null,
            "created_at": "1970-01-01T00:00:00Z",
            "updated_at": "1970-01-01T00:00:00Z",
            "message_count": 0
        });

        let result = serde_json::from_value::<ThreadMetadata>(value);

        assert!(result.is_err());
    }

    #[test]
    fn thread_metadata_rejects_blank_or_relative_workspace_when_deserializing() {
        for workspace in ["", "project"] {
            let value = serde_json::json!({
                "thread_id": "thread-a",
                "title": null,
                "workspace": workspace,
                "provider": null,
                "model": null,
                "created_at": "1970-01-01T00:00:00Z",
                "updated_at": "1970-01-01T00:00:00Z",
                "message_count": 0
            });

            let result = serde_json::from_value::<ThreadMetadata>(value);

            assert!(result.is_err(), "workspace {workspace:?} should fail");
        }
    }

    #[test]
    fn thread_usage_metadata_accumulates_cache_hit_rate() {
        let mut usage = ThreadUsageMetadata::default();

        usage.add_token_usage(&TokenUsage::new(100, 10, 110).with_cached_prompt_tokens(92));
        usage.add_token_usage(&TokenUsage::new(50, 5, 55).with_cached_prompt_tokens(43));

        assert_eq!(usage.prompt_tokens, 150);
        assert_eq!(usage.cached_prompt_tokens, 135);
        assert!((usage.cache_hit_rate.unwrap() - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn thread_item_events_replay_reasoning_and_final_answer_into_stable_items() {
        let timestamp = OffsetDateTime::UNIX_EPOCH;
        let events = vec![
            ThreadItemEvent {
                seq: 1,
                event_id: "event-1".to_string(),
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                timestamp,
                event: ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::Reasoning {
                        id: "turn-1-agent-reasoning".to_string(),
                        summary: Vec::new(),
                        content: vec![String::new()],
                        status: Some(ThreadItemStatus::InProgress),
                    },
                },
            },
            ThreadItemEvent {
                seq: 2,
                event_id: "event-2".to_string(),
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                timestamp,
                event: ThreadItemEventKind::ItemDelta {
                    item_id: "turn-1-agent-reasoning".to_string(),
                    delta: ThreadItemDelta::ReasoningText {
                        delta: "Inspecting".to_string(),
                        content_index: 0,
                    },
                },
            },
            ThreadItemEvent {
                seq: 3,
                event_id: "event-3".to_string(),
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                timestamp,
                event: ThreadItemEventKind::ItemDelta {
                    item_id: "turn-1-agent-final_answer".to_string(),
                    delta: ThreadItemDelta::AgentMessageText {
                        delta: "Done".to_string(),
                        phase: Some("final_answer".to_string()),
                    },
                },
            },
            ThreadItemEvent {
                seq: 4,
                event_id: "event-4".to_string(),
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                timestamp,
                event: ThreadItemEventKind::ItemCompleted {
                    item: ThreadItem::AgentMessage {
                        id: "turn-1-agent-final_answer".to_string(),
                        text: "Done.".to_string(),
                        phase: Some("final_answer".to_string()),
                        status: Some(ThreadItemStatus::Completed),
                    },
                },
            },
        ];

        let turns = project_thread_item_events(&events);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_id, "turn-1");
        assert_eq!(
            turns[0].items,
            vec![
                ThreadItem::Reasoning {
                    id: "turn-1-agent-reasoning".to_string(),
                    summary: Vec::new(),
                    content: vec!["Inspecting".to_string()],
                    status: Some(ThreadItemStatus::InProgress),
                },
                ThreadItem::AgentMessage {
                    id: "turn-1-agent-final_answer".to_string(),
                    text: "Done.".to_string(),
                    phase: Some("final_answer".to_string()),
                    status: Some(ThreadItemStatus::Completed),
                }
            ]
        );
    }

    #[test]
    fn inference_delta_public_item_events_start_item_before_delta() {
        let reasoning = public_item_event_kinds_from_inference_event(
            "turn-1",
            &InferenceEvent::ReasoningDelta(ReasoningDelta {
                text: "thinking".to_string(),
            }),
            false,
        );

        assert_eq!(
            reasoning,
            vec![
                ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::Reasoning {
                        id: "turn-1-agent-reasoning".to_string(),
                        summary: Vec::new(),
                        content: vec![String::new()],
                        status: Some(ThreadItemStatus::InProgress),
                    },
                },
                ThreadItemEventKind::ItemDelta {
                    item_id: "turn-1-agent-reasoning".to_string(),
                    delta: ThreadItemDelta::ReasoningText {
                        delta: "thinking".to_string(),
                        content_index: 0,
                    },
                },
            ]
        );

        let agent = public_item_event_kinds_from_inference_event(
            "turn-1",
            &InferenceEvent::MessageDelta(MessageDelta {
                text: "hello".to_string(),
                phase: Some("final_answer".to_string()),
            }),
            false,
        );

        assert_eq!(
            agent,
            vec![
                ThreadItemEventKind::ItemStarted {
                    item: ThreadItem::AgentMessage {
                        id: "turn-1-agent-final_answer".to_string(),
                        text: String::new(),
                        phase: Some("final_answer".to_string()),
                        status: Some(ThreadItemStatus::InProgress),
                    },
                },
                ThreadItemEventKind::ItemDelta {
                    item_id: "turn-1-agent-final_answer".to_string(),
                    delta: ThreadItemDelta::AgentMessageText {
                        delta: "hello".to_string(),
                        phase: Some("final_answer".to_string()),
                    },
                },
            ]
        );
    }
}
