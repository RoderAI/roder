use serde::{Deserialize, Serialize};

use crate::events::{ThreadId, TurnId};
use crate::inference::TokenUsage;

pub type SubagentTraceId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ParentTurnRef {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentTraceStatus {
    Queued,
    Running,
    WaitingForApproval,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentDestination {
    pub kind: SubagentDestinationKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentDestinationKind {
    InProcess,
    LocalWorktree,
    RemoteRunner,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentTraceSummary {
    pub trace_id: SubagentTraceId,
    pub parent: ParentTurnRef,
    pub child_thread_id: ThreadId,
    pub child_turn_id: TurnId,
    pub title: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub status: SubagentTraceStatus,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<SubagentDestination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentTraceDelta {
    pub trace_id: SubagentTraceId,
    pub parent: ParentTurnRef,
    pub item: SubagentTraceItem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SubagentTraceItem {
    Message {
        role: String,
        content: PagedTraceText,
    },
    Reasoning {
        content: PagedTraceText,
    },
    ToolCall {
        tool_id: String,
        tool_name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    ToolResult {
        tool_id: String,
        is_error: bool,
        output: PagedTraceText,
    },
    Status {
        status: SubagentTraceStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PagedTraceText {
    pub text: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

impl PagedTraceText {
    pub fn capped(text: impl Into<String>, max_chars: usize) -> Self {
        let text = text.into();
        let char_count = text.chars().count();
        if char_count <= max_chars {
            return Self {
                text,
                truncated: false,
                next_offset: None,
            };
        }
        Self {
            text: text.chars().take(max_chars).collect(),
            truncated: true,
            next_offset: Some(max_chars),
        }
    }
}

#[async_trait::async_trait]
pub trait SubagentTraceSink: Send + Sync + 'static {
    async fn trace_created(&self, summary: SubagentTraceSummary);

    async fn trace_delta(&self, delta: SubagentTraceDelta);

    async fn trace_status_changed(
        &self,
        trace_id: SubagentTraceId,
        parent: ParentTurnRef,
        status: SubagentTraceStatus,
        detail: Option<String>,
    );

    async fn trace_completed(&self, summary: SubagentTraceSummary);

    async fn trace_failed(&self, summary: SubagentTraceSummary, error: String);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_trace_summary_round_trips_camel_case_fields() {
        let summary = SubagentTraceSummary {
            trace_id: "trace-1".to_string(),
            parent: ParentTurnRef {
                thread_id: "parent-thread".to_string(),
                turn_id: "parent-turn".to_string(),
            },
            child_thread_id: "child-thread".to_string(),
            child_turn_id: "child-turn".to_string(),
            title: "Inspect files".to_string(),
            role: "explorer".to_string(),
            model: Some("gpt-test".to_string()),
            status: SubagentTraceStatus::Running,
            elapsed_ms: 1200,
            usage: None,
            destination: Some(SubagentDestination {
                kind: SubagentDestinationKind::InProcess,
                label: "workspace".to_string(),
                path: None,
                provider_id: None,
                destination_id: None,
            }),
            latest_activity: Some("reading README".to_string()),
            error_summary: None,
        };

        let value = serde_json::to_value(&summary).unwrap();
        assert_eq!(value["traceId"], "trace-1");
        assert_eq!(value["childThreadId"], "child-thread");
        assert_eq!(value["status"], "running");
        assert_eq!(value["destination"]["kind"], "in_process");

        let round_trip: SubagentTraceSummary = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip, summary);
    }

    #[test]
    fn subagent_trace_delta_caps_tool_output() {
        let output = PagedTraceText::capped("abcdef", 3);

        assert_eq!(output.text, "abc");
        assert!(output.truncated);
        assert_eq!(output.next_offset, Some(3));
    }
}
