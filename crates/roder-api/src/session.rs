use crate::events::{EventEnvelope, ThreadId};
use std::sync::Arc;

#[async_trait::async_trait]
pub trait SessionStore: Send + Sync {
    async fn append_event(&self, thread_id: &ThreadId, envelope: &EventEnvelope) -> anyhow::Result<()>;
    async fn load_events(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<EventEnvelope>>;
}

pub trait SessionStoreFactory: Send + Sync + 'static {
    fn create(&self) -> Arc<dyn SessionStore>;
}

pub trait CheckpointStoreFactory: Send + Sync + 'static {}
