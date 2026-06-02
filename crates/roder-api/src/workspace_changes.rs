use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceChangeStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceChangeSource {
    VersionControlReconciled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceChangeConfidence {
    ObservedAfterTool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceObservedFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: WorkspaceChangeStatus,
    pub additions: u32,
    pub deletions: u32,
    #[serde(default)]
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChangeObservation {
    pub id: String,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_call_id: String,
    pub tool_name: String,
    pub source: WorkspaceChangeSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<crate::version_control::VcsProviderId>,
    pub confidence: WorkspaceChangeConfidence,
    pub files: Vec<WorkspaceObservedFile>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_control_reconciled_observation_round_trips_with_provider_identity() {
        let observation = WorkspaceChangeObservation {
            id: "obs-1".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            tool_call_id: "tool-1".to_string(),
            tool_name: "shell".to_string(),
            source: WorkspaceChangeSource::VersionControlReconciled,
            provider_id: Some("git".to_string()),
            confidence: WorkspaceChangeConfidence::ObservedAfterTool,
            files: vec![WorkspaceObservedFile {
                path: "src/lib.rs".to_string(),
                old_path: None,
                status: WorkspaceChangeStatus::Modified,
                additions: 2,
                deletions: 1,
                binary: false,
            }],
            created_at: OffsetDateTime::UNIX_EPOCH,
        };

        let encoded = serde_json::to_value(&observation).expect("serialize observation");
        let decoded = serde_json::from_value::<WorkspaceChangeObservation>(encoded)
            .expect("deserialize observation");

        assert_eq!(decoded, observation);
    }
}
