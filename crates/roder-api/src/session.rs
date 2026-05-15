use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::conversation::TurnItem;
use crate::events::{EventEnvelope, ThreadId, TurnId};
pub use crate::extension::{CheckpointStoreId, SessionStoreId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMetadata {
    pub thread_id: ThreadId,
    pub title: Option<String>,
    pub workspace: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub message_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnRecord {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub items: Vec<TurnItem>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadSnapshot {
    pub metadata: Option<SessionMetadata>,
    pub events: Vec<EventEnvelope>,
    pub turns: Vec<TurnRecord>,
}

#[async_trait::async_trait]
pub trait SessionStore: Send + Sync {
    fn id(&self) -> SessionStoreId;

    async fn create_session(&self, metadata: SessionMetadata) -> anyhow::Result<SessionMetadata>;
    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionMetadata>>;
    async fn load_session(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>>;
    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()>;
    async fn append_turn_item(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        item: &TurnItem,
    ) -> anyhow::Result<()>;
}

pub trait SessionStoreFactory: Send + Sync + 'static {
    fn id(&self) -> SessionStoreId;
    fn create(&self) -> Arc<dyn SessionStore>;
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
    fn session_metadata_timestamps_serialize_as_rfc3339_strings() {
        let value = serde_json::to_value(SessionMetadata {
            thread_id: "thread-a".to_string(),
            title: None,
            workspace: None,
            provider: None,
            model: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            message_count: 0,
        })
        .unwrap();

        assert_eq!(value["created_at"], "1970-01-01T00:00:00Z");
        assert_eq!(value["updated_at"], "1970-01-01T00:00:00Z");
    }
}
