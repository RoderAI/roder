//! Typed analytics records and query filters (roadmap phase 73).
//!
//! Records deliberately exclude prompt bodies, assistant text, tool output
//! bodies, command payloads, and secrets. Tool names, provider/model ids,
//! ids, timestamps, status, durations, usage counts, and bounded error
//! classes are the entire vocabulary.

use serde::{Deserialize, Serialize};

/// Version stamped on exported/imported normalized JSONL records.
pub const ANALYTICS_JSONL_SCHEMA_VERSION: u32 = 1;

/// How workspace paths are recorded and reported.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceLabelMode {
    /// Record the full local path (local-only default).
    #[default]
    FullPath,
    /// Record an FNV-1a hash of the path.
    Hashed,
    /// Record only the final path component.
    BasenameOnly,
}

impl WorkspaceLabelMode {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "full_path" => Ok(Self::FullPath),
            "hashed" => Ok(Self::Hashed),
            "basename_only" => Ok(Self::BasenameOnly),
            other => anyhow::bail!(
                "unknown workspace label mode {other:?}; expected full_path, hashed, or \
                 basename_only"
            ),
        }
    }

    /// Stable grouping key plus display label for a workspace path.
    pub fn label(&self, workspace: &str) -> (String, String) {
        match self {
            WorkspaceLabelMode::FullPath => (workspace.to_string(), workspace.to_string()),
            WorkspaceLabelMode::Hashed => {
                let hash = fnv1a(workspace.as_bytes());
                (hash.clone(), hash)
            }
            WorkspaceLabelMode::BasenameOnly => {
                let basename = workspace
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(workspace)
                    .to_string();
                (fnv1a(workspace.as_bytes()), basename)
            }
        }
    }
}

pub(crate) fn fnv1a(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub thread_id: String,
    pub workspace_key: Option<String>,
    pub workspace_label: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnRecord {
    pub thread_id: String,
    pub turn_id: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub runtime_profile: Option<String>,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    /// `running`, `completed`, `failed`, or `partial`.
    pub status: String,
    pub error_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageRecord {
    pub thread_id: String,
    pub turn_id: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub recorded_at_ms: i64,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub cached_prompt_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRecord {
    pub thread_id: String,
    pub turn_id: String,
    pub tool_id: String,
    pub tool_name: Option<String>,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    /// `running`, `success`, `error`, or `partial` (missing start event).
    pub status: String,
    pub is_error: bool,
}

/// Common filter for stats queries. All bounds are optional; `since_ms`
/// and `until_ms` are inclusive/exclusive epoch-millisecond bounds.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatsFilter {
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    pub workspace_key: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub thread_id: Option<String>,
    pub tool_name: Option<String>,
    pub min_calls: Option<u64>,
    /// Maximum rows returned by listing queries (default applied by query
    /// helpers; app-server callers must bound this).
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSummary {
    pub tool_name: String,
    pub call_count: u64,
    pub error_count: u64,
    pub error_rate: f64,
    pub total_duration_ms: i64,
    pub avg_duration_ms: Option<f64>,
    pub p50_duration_ms: Option<i64>,
    pub p95_duration_ms: Option<i64>,
    pub p99_duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenSummaryRow {
    /// Group key: a day (`YYYY-MM-DD`), thread id, provider, model, or
    /// workspace key depending on the requested grouping.
    pub group: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cached_prompt_tokens: u64,
    pub turn_count: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenGroup {
    Day,
    Session,
    Provider,
    Model,
    Workspace,
}

impl TokenGroup {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "day" => Ok(Self::Day),
            "session" => Ok(Self::Session),
            "provider" => Ok(Self::Provider),
            "model" => Ok(Self::Model),
            "workspace" => Ok(Self::Workspace),
            other => anyhow::bail!(
                "unknown token grouping {other:?}; expected day, session, provider, model, or \
                 workspace"
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub thread_id: String,
    pub workspace_label: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub turn_count: u64,
    pub tool_call_count: u64,
    pub tool_error_count: u64,
    pub total_tokens: u64,
    pub total_tool_duration_ms: i64,
    pub first_activity_ms: Option<i64>,
    pub last_activity_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub turn_count: u64,
    pub completed_turn_count: u64,
    pub failed_turn_count: u64,
    pub tool_call_count: u64,
    pub tool_error_count: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cached_prompt_tokens: u64,
    pub session_count: u64,
    pub most_called_tool: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DailyRollupRow {
    pub day: String,
    pub workspace_key: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tool_name: Option<String>,
    pub call_count: u64,
    pub error_count: u64,
    pub total_duration_ms: i64,
    pub p50_duration_ms: Option<i64>,
    pub p95_duration_ms: Option<i64>,
    pub p99_duration_ms: Option<i64>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cached_prompt_tokens: u64,
}
