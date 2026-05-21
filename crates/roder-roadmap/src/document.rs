use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub goal: String,
    pub architecture: String,
    pub tech_stack: String,
    pub owned_paths: Vec<String>,
    pub tasks: Vec<Task>,
    pub acceptance: Vec<ChecklistItem>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub checked_tasks: usize,
    pub unchecked_tasks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub heading: String,
    pub checked: bool,
    pub line: usize,
    pub level: usize,
    pub body_range: LineRange,
    pub run_blocks: Vec<String>,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: String,
    pub text: String,
    pub checked: bool,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationResult {
    pub document_id: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadAttachment {
    pub thread_id: String,
    pub task_id: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoadmapState {
    pub document_id: String,
    pub path: PathBuf,
    pub focused_task_id: Option<String>,
    pub primary_thread_id: Option<String>,
    pub attached_thread_id: Option<String>,
    pub threads: Vec<ThreadAttachment>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_validation: Option<OffsetDateTime>,
    pub last_diagnostics: Vec<Diagnostic>,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}
