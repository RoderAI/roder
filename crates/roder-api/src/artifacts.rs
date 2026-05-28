use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};

pub type ContextArtifactId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    pub fn as_str(&self) -> &'static str {
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
pub struct ContextArtifact {
    pub id: ContextArtifactId,
    pub kind: ContextArtifactKind,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub byte_count: u64,
    pub line_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub store_path: String,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub retention_expires_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(default = "default_roder_owned")]
    pub roder_owned: bool,
}

impl ContextArtifact {
    pub fn descriptor(&self) -> ContextArtifactDescriptor {
        ContextArtifactDescriptor {
            id: self.id.clone(),
            kind: self.kind.clone(),
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
            byte_count: self.byte_count,
            line_count: self.line_count,
            source_tool_id: self.source_tool_id.clone(),
            label: self.label.clone(),
            retention_expires_at: self.retention_expires_at,
            created_at: self.created_at,
        }
    }
}

fn default_roder_owned() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextArtifactDescriptor {
    pub id: ContextArtifactId,
    pub kind: ContextArtifactKind,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub byte_count: u64,
    pub line_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub retention_expires_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactReadPage {
    pub artifact: ContextArtifactDescriptor,
    pub text: String,
    pub start_line: usize,
    pub limit: usize,
    pub shown: usize,
    pub total_lines: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_start_line: Option<usize>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct CreateArtifactRequest<'a> {
    pub kind: ContextArtifactKind,
    pub thread_id: &'a ThreadId,
    pub turn_id: &'a TurnId,
    pub source_tool_id: Option<&'a str>,
    pub label: Option<&'a str>,
    pub bytes: &'a [u8],
}

#[derive(Clone)]
pub struct ContextArtifactStore {
    backend: Arc<dyn ContextArtifactAccess>,
}

impl ContextArtifactStore {
    pub fn new(backend: Arc<dyn ContextArtifactAccess>) -> Self {
        Self { backend }
    }

    pub fn backend(&self) -> Arc<dyn ContextArtifactAccess> {
        Arc::clone(&self.backend)
    }

    pub fn create(&self, request: CreateArtifactRequest<'_>) -> anyhow::Result<ContextArtifact> {
        self.backend.create_artifact(request)
    }

    pub fn append(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        bytes: &[u8],
    ) -> anyhow::Result<ContextArtifact> {
        self.backend.append_artifact(thread_id, artifact_id, bytes)
    }

    pub fn list_artifacts(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<ContextArtifact>> {
        self.backend.list_artifacts(thread_id)
    }

    pub fn read_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        start_line: usize,
        limit: usize,
    ) -> anyhow::Result<ArtifactReadPage> {
        self.backend
            .read_artifact(thread_id, artifact_id, start_line, limit)
    }

    pub fn grep_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<ArtifactGrepPage> {
        self.backend
            .grep_artifact(thread_id, artifact_id, query, offset, limit)
    }

    pub fn tail_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        lines: usize,
    ) -> anyhow::Result<ArtifactTailPage> {
        self.backend.tail_artifact(thread_id, artifact_id, lines)
    }

    pub fn delete_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
    ) -> anyhow::Result<bool> {
        self.backend.delete_artifact(thread_id, artifact_id)
    }
}

impl std::fmt::Debug for ContextArtifactStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextArtifactStore")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactGrepPage {
    pub artifact: ContextArtifactDescriptor,
    pub query: String,
    pub text: String,
    pub offset: usize,
    pub limit: usize,
    pub shown: usize,
    pub total_matches: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactTailPage {
    pub artifact: ContextArtifactDescriptor,
    pub text: String,
    pub start_line: usize,
    pub lines: usize,
    pub shown: usize,
    pub total_lines: usize,
    pub truncated: bool,
}

pub trait ContextArtifactAccess: Send + Sync + 'static {
    fn create_artifact(
        &self,
        request: CreateArtifactRequest<'_>,
    ) -> anyhow::Result<ContextArtifact>;
    fn append_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        bytes: &[u8],
    ) -> anyhow::Result<ContextArtifact>;
    fn list_artifacts(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<ContextArtifact>>;
    fn read_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        start_line: usize,
        limit: usize,
    ) -> anyhow::Result<ArtifactReadPage>;
    fn grep_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<ArtifactGrepPage>;
    fn tail_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        lines: usize,
    ) -> anyhow::Result<ArtifactTailPage>;
    fn delete_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
    ) -> anyhow::Result<bool>;
}

pub fn format_artifact_reference(artifact: &ContextArtifact, label: impl AsRef<str>) -> String {
    let label = label.as_ref();
    let label = if label.is_empty() { "content" } else { label };
    format!(
        "[artifact: {} {label} lines={} bytes={} id={}]\nUse read_artifact, grep_artifact, or tail_artifact with artifact_id \"{}\" to inspect more.",
        artifact.kind.as_str(),
        artifact.line_count,
        artifact.byte_count,
        artifact.id,
        artifact.id
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_descriptor_hides_store_path() {
        let artifact = ContextArtifact {
            id: "artifact-1".to_string(),
            kind: ContextArtifactKind::ToolOutput,
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            byte_count: 10,
            line_count: 2,
            source_tool_id: Some("call-1".to_string()),
            label: Some("stdout".to_string()),
            store_path: "/tmp/private/artifact-1.txt".to_string(),
            retention_expires_at: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            roder_owned: true,
        };

        let value = serde_json::to_value(artifact.descriptor()).unwrap();

        assert_eq!(value["kind"], "tool_output");
        assert_eq!(value["sourceToolId"], "call-1");
        assert!(value.get("storePath").is_none());
    }

    #[test]
    fn artifact_reference_names_follow_up_tools() {
        let artifact = ContextArtifact {
            id: "artifact-1".to_string(),
            kind: ContextArtifactKind::CommandStdout,
            thread_id: "app-server".to_string(),
            turn_id: "process-1".to_string(),
            byte_count: 12,
            line_count: 1,
            source_tool_id: Some("process-1".to_string()),
            label: Some("stdout".to_string()),
            store_path: "/tmp/private/artifact-1.txt".to_string(),
            retention_expires_at: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            roder_owned: true,
        };

        let reference = format_artifact_reference(&artifact, "stdout");

        assert!(
            reference.contains("[artifact: command_stdout stdout lines=1 bytes=12 id=artifact-1]")
        );
        assert!(reference.contains("read_artifact"));
        assert!(reference.contains("grep_artifact"));
    }
}
