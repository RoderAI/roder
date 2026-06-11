//! `stats/*` protocol DTOs for local usage analytics (roadmap phase 73).
//!
//! Responses re-use the analytics crate's summary records, which exclude
//! prompt/assistant/tool-output bodies and secrets by construction.
//! Workspace labels arrive already transformed by the configured label
//! mode. All listing windows are bounded server-side.

use serde::{Deserialize, Serialize};

pub use roder_usage_analytics::{
    SessionSummary, StatsFilter, TokenSummaryRow, ToolSummary, UsageSummary,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsQueryParams {
    #[serde(default)]
    pub filter: StatsFilter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSummaryResult {
    pub summary: UsageSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsToolsParams {
    #[serde(default)]
    pub filter: StatsFilter,
    /// `calls` (default), `p95`, `errors`, or `underused`.
    #[serde(default)]
    pub sort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsToolsResult {
    pub tools: Vec<ToolSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsTokensParams {
    #[serde(default)]
    pub filter: StatsFilter,
    /// `day` (default), `session`, `provider`, `model`, or `workspace`.
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsTokensResult {
    pub rows: Vec<TokenSummaryRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSessionsResult {
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsBackfillParams {
    #[serde(default)]
    pub rebuild: bool,
    #[serde(default)]
    pub best_effort: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsBackfillResult {
    pub files_scanned: u64,
    pub lines_ingested: u64,
    pub lines_skipped_by_offset: u64,
    pub sessions_enriched: u64,
    pub parse_error_count: u64,
    pub rollup_rows: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsExportParams {
    /// Server-side output path; export never streams an unbounded payload
    /// through the JSON-RPC response.
    pub output_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsExportResult {
    pub output_path: String,
    pub records: u64,
}
