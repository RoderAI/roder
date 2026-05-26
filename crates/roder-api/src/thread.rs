use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};
use time::OffsetDateTime;

use crate::events::{EventEnvelope, ThreadId, TurnId};
pub use crate::extension::{CheckpointStoreId, ThreadStoreId};
use crate::extension_state::ExtensionStateRecord;
use crate::remote_runner::{RunnerDestination, RunnerSessionState};
use crate::transcript::TranscriptItem;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadMetadata {
    pub thread_id: ThreadId,
    pub title: Option<String>,
    #[serde(deserialize_with = "deserialize_thread_workspace")]
    pub workspace: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_destination: Option<RunnerDestination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_state: Option<RunnerSessionState>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub message_count: u32,
}

pub fn validate_thread_workspace(workspace: &str) -> anyhow::Result<String> {
    let workspace = workspace.trim();
    anyhow::ensure!(!workspace.is_empty(), "thread workspace is required");
    anyhow::ensure!(
        std::path::Path::new(workspace).is_absolute(),
        "thread workspace must be an absolute path: {workspace}"
    );
    Ok(workspace.to_string())
}

fn deserialize_thread_workspace<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let workspace = String::deserialize(deserializer)?;
    validate_thread_workspace(&workspace).map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnRecord {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub items: Vec<TranscriptItem>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadSnapshot {
    pub metadata: Option<ThreadMetadata>,
    pub events: Vec<EventEnvelope>,
    pub turns: Vec<TurnRecord>,
    pub extension_states: Vec<ExtensionStateRecord>,
}

#[async_trait::async_trait]
pub trait ThreadStore: Send + Sync {
    fn id(&self) -> ThreadStoreId;

    fn local_thread_root(&self) -> Option<PathBuf> {
        None
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata>;
    async fn update_thread_metadata(
        &self,
        metadata: ThreadMetadata,
    ) -> anyhow::Result<ThreadMetadata> {
        Ok(metadata)
    }
    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>>;
    async fn load_thread(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>>;
    async fn archive_thread(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        let _ = thread_id;
        anyhow::bail!("thread store {} does not support archive", self.id())
    }
    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()>;
    async fn append_turn_item(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        item: &TranscriptItem,
    ) -> anyhow::Result<()>;
    async fn append_extension_state(
        &self,
        thread_id: &ThreadId,
        record: &ExtensionStateRecord,
    ) -> anyhow::Result<()> {
        let _ = (thread_id, record);
        anyhow::bail!(
            "thread store {} does not support extension state",
            self.id()
        )
    }
}

pub trait ThreadStoreFactory: Send + Sync + 'static {
    fn id(&self) -> ThreadStoreId;
    fn create(&self) -> Arc<dyn ThreadStore>;
}

#[async_trait::async_trait]
pub trait CheckpointStore: Send + Sync {
    fn id(&self) -> CheckpointStoreId;
    async fn save_snapshot(&self, snapshot: ThreadSnapshot) -> anyhow::Result<()>;
    async fn load_snapshot(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>>;
}

pub trait CheckpointStoreFactory: Send + Sync + 'static {
    fn id(&self) -> CheckpointStoreId;
    fn create(&self) -> Arc<dyn CheckpointStore>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_metadata_timestamps_serialize_as_rfc3339_strings() {
        let value = serde_json::to_value(ThreadMetadata {
            thread_id: "thread-a".to_string(),
            title: None,
            workspace: "/workspace".to_string(),
            provider: None,
            model: None,
            runner_destination: None,
            runner_state: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            message_count: 0,
        })
        .unwrap();

        assert_eq!(value["created_at"], "1970-01-01T00:00:00Z");
        assert_eq!(value["updated_at"], "1970-01-01T00:00:00Z");
        assert_eq!(value["workspace"], "/workspace");
    }

    #[test]
    fn thread_metadata_requires_workspace_when_deserializing() {
        let value = serde_json::json!({
            "thread_id": "thread-a",
            "title": null,
            "provider": null,
            "model": null,
            "created_at": "1970-01-01T00:00:00Z",
            "updated_at": "1970-01-01T00:00:00Z",
            "message_count": 0
        });

        let result = serde_json::from_value::<ThreadMetadata>(value);

        assert!(result.is_err());
    }

    #[test]
    fn thread_metadata_rejects_blank_or_relative_workspace_when_deserializing() {
        for workspace in ["", "project"] {
            let value = serde_json::json!({
                "thread_id": "thread-a",
                "title": null,
                "workspace": workspace,
                "provider": null,
                "model": null,
                "created_at": "1970-01-01T00:00:00Z",
                "updated_at": "1970-01-01T00:00:00Z",
                "message_count": 0
            });

            let result = serde_json::from_value::<ThreadMetadata>(value);

            assert!(result.is_err(), "workspace {workspace:?} should fail");
        }
    }
}
