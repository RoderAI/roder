use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::extension::{EmbeddingProviderId, MemoryStoreId};

pub type MemoryId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryScope {
    Global,
    User(String),
    Workspace(String),
    Project(String),
    Thread(String),
}

impl MemoryScope {
    pub fn stable_id(&self) -> String {
        match self {
            MemoryScope::Global => "global".to_string(),
            MemoryScope::User(id) => format!("user:{id}"),
            MemoryScope::Workspace(id) => format!("workspace:{id}"),
            MemoryScope::Project(id) => format!("project:{id}"),
            MemoryScope::Thread(id) => format!("thread:{id}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryScopeDescriptor {
    pub id: String,
    pub scope: MemoryScope,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUsageMetadata {
    pub use_count: u64,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCitation {
    pub memory_id: MemoryId,
    pub scope_id: String,
    pub snippet: String,
    pub score_millis: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecord {
    pub id: Option<MemoryId>,
    pub scope: MemoryScope,
    pub text: String,
    #[serde(default)]
    pub content_hash: Option<String>,
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub usage: Option<MemoryUsageMetadata>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryQuery {
    pub scope: Option<MemoryScope>,
    pub text: String,
    pub limit: usize,
    #[serde(default)]
    pub include_global: bool,
    #[serde(default)]
    pub provider_id: Option<EmbeddingProviderId>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchResult {
    pub record: MemoryRecord,
    pub score: f32,
    #[serde(default)]
    pub citation: Option<MemoryCitation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemorySaveRequest {
    pub scope: MemoryScope,
    pub text: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUpdateRequest {
    pub id: MemoryId,
    pub text: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProviderSelection {
    pub provider_id: EmbeddingProviderId,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryJobLease {
    pub id: String,
    pub scope_id: String,
    pub provider_id: EmbeddingProviderId,
    pub model: String,
    #[serde(with = "time::serde::rfc3339")]
    pub leased_until: OffsetDateTime,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    fn id(&self) -> MemoryStoreId;

    async fn put(&self, record: MemoryRecord) -> anyhow::Result<MemoryId>;
    async fn get(&self, id: &MemoryId) -> anyhow::Result<Option<MemoryRecord>>;
    async fn search(&self, query: MemoryQuery) -> anyhow::Result<Vec<MemorySearchResult>>;
    async fn delete(&self, id: &MemoryId) -> anyhow::Result<()>;
    async fn list(
        &self,
        scope: Option<MemoryScope>,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryRecord>> {
        let text = String::new();
        let results = self
            .search(MemoryQuery {
                scope,
                text,
                limit,
                include_global: false,
                provider_id: None,
                model: None,
            })
            .await?;
        Ok(results.into_iter().map(|result| result.record).collect())
    }
}

pub trait MemoryStoreFactory: Send + Sync + 'static {
    fn id(&self) -> MemoryStoreId;
    fn create(&self) -> Arc<dyn MemoryStore>;
}
