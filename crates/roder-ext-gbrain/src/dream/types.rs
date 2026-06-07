use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamStatus {
    Running,
    Completed,
    Failed,
    Canceled,
}

impl DreamStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamMode {
    Enrich,
    Refine,
    Compact,
    Full,
}

impl DreamMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enrich => "enrich",
            Self::Refine => "refine",
            Self::Compact => "compact",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamPolicy {
    Interactive,
    Eval,
    Import,
    Maintenance,
}

impl DreamPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Eval => "eval",
            Self::Import => "import",
            Self::Maintenance => "maintenance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConfidenceLabel {
    Extracted,
    Inferred,
    Ambiguous,
}

impl ConfidenceLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Extracted => "EXTRACTED",
            Self::Inferred => "INFERRED",
            Self::Ambiguous => "AMBIGUOUS",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamRun {
    pub id: String,
    pub scope_id: String,
    pub mode: DreamMode,
    pub status: DreamStatus,
    pub algorithm_version: String,
    pub run_policy: DreamPolicy,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoner_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub confidence: ConfidenceLabel,
    #[serde(default)]
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_fact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dream_run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub source_node_id: String,
    pub target_node_id: String,
    pub relation: String,
    pub confidence: ConfidenceLabel,
    #[serde(default)]
    pub directed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dream_run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphHyperedge {
    pub id: String,
    pub kind: String,
    pub node_ids: Vec<String>,
    pub confidence: ConfidenceLabel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dream_run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceCard {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub confidence: ConfidenceLabel,
    #[serde(default)]
    pub source_fact_ids: Vec<String>,
    #[serde(default)]
    pub quote_spans: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemporalEvent {
    pub id: String,
    pub temporal_class: String,
    pub confidence: ConfidenceLabel,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub valid_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub invalid_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_statement_id: Option<String>,
}
