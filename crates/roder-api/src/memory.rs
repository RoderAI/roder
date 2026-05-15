use serde::{Serialize, Deserialize};

pub type MemoryId = String;
pub type MemoryScope = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: Option<MemoryId>,
    pub scope: MemoryScope,
    pub text: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct MemoryQuery {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct MemorySearchResult {
    pub record: MemoryRecord,
    pub score: f32,
}

#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    async fn put(&self, record: MemoryRecord) -> anyhow::Result<MemoryId>;
    async fn get(&self, id: MemoryId) -> anyhow::Result<Option<MemoryRecord>>;
    async fn search(
        &self,
        query: MemoryQuery,
    ) -> anyhow::Result<Vec<MemorySearchResult>>;
    async fn delete(&self, id: MemoryId) -> anyhow::Result<()>;
}

pub trait MemoryStoreFactory: Send + Sync + 'static {}
