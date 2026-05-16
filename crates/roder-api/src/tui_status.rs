use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::policy_mode::PolicyMode;

pub type StatusSegmentId = String;
pub type PaletteSourceId = String;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaletteSourceDescriptor {
    pub id: PaletteSourceId,
    pub label: String,
    pub priority: i32,
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

pub fn built_in_status_segments() -> Vec<StatusSegment> {
    vec![
        StatusSegment::new("mode", 100, 8, |ctx| StatusCell {
            text: format!("mode:{}", policy_mode_label(ctx.policy_mode)),
            style: StatusStyle::Accent,
            tooltip: Some("Active policy mode".to_string()),
        }),
        StatusSegment::new("model", 90, 8, |ctx| StatusCell {
            text: ctx
                .model
                .map(|model| format!("model:{model}"))
                .unwrap_or_else(|| "model:-".to_string()),
            style: StatusStyle::Default,
            tooltip: Some("Active model".to_string()),
        }),
        StatusSegment::new("session", 80, 8, |ctx| StatusCell {
            text: format!("session:{}", short_id(&ctx.session.thread_id)),
            style: StatusStyle::Muted,
            tooltip: ctx.session.title.clone(),
        }),
        StatusSegment::new("branch", 70, 8, |ctx| StatusCell {
            text: ctx
                .git
                .and_then(|git| git.branch.as_deref())
                .map(|branch| format!("branch:{branch}"))
                .unwrap_or_else(|| "branch:-".to_string()),
            style: StatusStyle::Muted,
            tooltip: Some("Best-effort git branch".to_string()),
        }),
        StatusSegment::new("usage", 60, 8, |ctx| StatusCell {
            text: ctx
                .usage
                .map(|usage| format!("tok:{}", usage.input_tokens + usage.output_tokens))
                .unwrap_or_else(|| "tok:-".to_string()),
            style: StatusStyle::Muted,
            tooltip: Some("Session token usage".to_string()),
        }),
        StatusSegment::new("mcp", 50, 6, |ctx| StatusCell {
            text: format!("mcp:{}", ctx.mcp.len()),
            style: StatusStyle::Muted,
            tooltip: Some("Configured MCP servers".to_string()),
        }),
    ]
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn policy_mode_label(mode: PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Default => "default",
        PolicyMode::AcceptAll => "accept_all",
        PolicyMode::Plan => "plan",
        PolicyMode::Bypass => "bypass",
    }
}
