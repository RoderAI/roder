use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub type ContextArtifactId = String;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextArtifactKind {
    ToolOutput,
    CommandStdout,
    CommandStderr,
    TerminalTranscript,
    ChatHistory,
    CompactionSource,
    ContextProviderDump,
}

impl ContextArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolOutput => "tool_output",
            Self::CommandStdout => "command_stdout",
            Self::CommandStderr => "command_stderr",
            Self::TerminalTranscript => "terminal_transcript",
            Self::ChatHistory => "chat_history",
            Self::CompactionSource => "compaction_source",
            Self::ContextProviderDump => "context_provider_dump",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRetention {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextArtifact {
    pub id: ContextArtifactId,
    pub kind: ContextArtifactKind,
    pub thread_id: String,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tool_id: Option<String>,
    /// Path relative to the artifact store root (e.g. `thread-a/turn-b/call_123.stdout.txt`).
    pub relative_path: String,
    pub byte_size: u64,
    pub line_count: u64,
    pub retention: ArtifactRetention,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextArtifactReference {
    pub artifact_id: ContextArtifactId,
    pub kind: ContextArtifactKind,
    pub label: String,
    pub line_count: u64,
    pub relative_path: String,
}

impl ContextArtifactReference {
    pub fn from_artifact(artifact: &ContextArtifact, label: impl Into<String>) -> Self {
        Self {
            artifact_id: artifact.id.clone(),
            kind: artifact.kind,
            label: label.into(),
            line_count: artifact.line_count,
            relative_path: artifact.relative_path.clone(),
        }
    }
}

/// Model-visible artifact pointer with follow-up instructions.
pub fn format_artifact_reference(reference: &ContextArtifactReference) -> String {
    format!(
        "[artifact: {} {} {} lines={} path={}]\nUse read_artifact or grep_artifact to inspect more.",
        reference.kind.as_str(),
        reference.label,
        reference.artifact_id,
        reference.line_count,
        reference.relative_path
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactReadPage {
    pub artifact_id: ContextArtifactId,
    pub start_line: u64,
    pub line_count: u64,
    pub total_lines: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_start_line: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactGrepMatch {
    pub line_number: u64,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactGrepResult {
    pub artifact_id: ContextArtifactId,
    pub pattern: String,
    pub matches: Vec<ArtifactGrepMatch>,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_reference_format_matches_contract() {
        let reference = ContextArtifactReference {
            artifact_id: "call_123".to_string(),
            kind: ContextArtifactKind::ToolOutput,
            label: "stdout".to_string(),
            line_count: 842,
            relative_path: "turn-b/call_123_stdout.txt".to_string(),
        };
        let text = format_artifact_reference(&reference);
        assert!(text.contains("[artifact: tool_output stdout call_123 lines=842"));
        assert!(text.contains("read_artifact"));
    }
}
