use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::extension::MemoryStoreId;

pub type MemoryId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryScope {
    Global,
    User(String),
    Workspace(String),
    Project(String),
    Session(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRecord {
    pub id: Option<MemoryId>,
    pub scope: MemoryScope,
    pub text: String,
    pub metadata: serde_json::Value,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryQuery {
    pub scope: Option<MemoryScope>,
    pub text: String,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemorySearchResult {
    pub record: MemoryRecord,
    pub score: f32,
}

#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    fn id(&self) -> MemoryStoreId;

    async fn put(&self, record: MemoryRecord) -> anyhow::Result<MemoryId>;
    async fn get(&self, id: &MemoryId) -> anyhow::Result<Option<MemoryRecord>>;
    async fn search(&self, query: MemoryQuery) -> anyhow::Result<Vec<MemorySearchResult>>;
    async fn delete(&self, id: &MemoryId) -> anyhow::Result<()>;
}

pub trait MemoryStoreFactory: Send + Sync + 'static {
    fn id(&self) -> MemoryStoreId;
    fn create(&self) -> Arc<dyn MemoryStore>;
}
