use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::policy_mode::PolicyMode;

pub type StatusSegmentId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StatusStyle {
    Default,
    Muted,
    Accent,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusCell {
    pub text: String,
    pub style: StatusStyle,
    pub tooltip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub thread_id: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitSnapshot {
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerStatus {
    pub id: String,
    pub state: String,
}

pub struct StatusContext<'a> {
    pub session: &'a SessionSummary,
    pub policy_mode: PolicyMode,
    pub model: Option<&'a str>,
    pub usage: Option<&'a SessionUsage>,
    pub git: Option<&'a GitSnapshot>,
    pub mcp: &'a [McpServerStatus],
}

pub struct StatusSegment {
    pub id: StatusSegmentId,
    pub priority: i32,
    pub min_width: u16,
    pub render: Arc<dyn Fn(&StatusContext<'_>) -> StatusCell + Send + Sync>,
}

impl Clone for StatusSegment {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            priority: self.priority,
            min_width: self.min_width,
            render: Arc::clone(&self.render),
        }
    }
}

impl StatusSegment {
    pub fn new(
        id: impl Into<StatusSegmentId>,
        priority: i32,
        min_width: u16,
        render: impl Fn(&StatusContext<'_>) -> StatusCell + Send + Sync + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            priority,
            min_width,
            render: Arc::new(render),
        }
    }
}
