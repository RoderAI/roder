use std::path::PathBuf;
use std::sync::Arc;

use roder_api::context::{
    ContextBlock, ContextBlockKind, ContextProvider, ContextProviderId, ContextQuery,
};
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::memory::{
    MemoryId, MemoryQuery, MemoryRecord, MemoryScope, MemorySearchResult, MemoryStore,
    MemoryStoreFactory,
};
use semver::Version;
use tokio::fs;
use tokio::sync::RwLock;

pub struct MemoryExtension {
    base_path: PathBuf,
}

impl MemoryExtension {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }
}

impl RoderExtension for MemoryExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-memory".to_string(),
            name: "Local Memory".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Local disk memory store and context provider".to_string()),
            provides: vec![
                ProvidedService::MemoryStore("local-memory".to_string()),
                ProvidedService::ContextProvider("memory-context".to_string()),
            ],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        let factory = Arc::new(LocalMemoryStoreFactory {
            base_path: self.base_path.clone(),
        });
        let store = factory.create();
        registry.memory_store_factory(factory);
        registry.context_provider(Arc::new(MemoryContextProvider { store }));
        Ok(())
    }
}

pub struct LocalMemoryStoreFactory {
    base_path: PathBuf,
}

impl MemoryStoreFactory for LocalMemoryStoreFactory {
    fn id(&self) -> roder_api::extension::MemoryStoreId {
        "local-memory".to_string()
    }

    fn create(&self) -> Arc<dyn MemoryStore> {
        Arc::new(LocalMemoryStore {
            file_path: self.base_path.join("memories.jsonl"),
            records: RwLock::new(None),
        })
    }
}

pub struct LocalMemoryStore {
    file_path: PathBuf,
    records: RwLock<Option<Vec<MemoryRecord>>>,
}

impl LocalMemoryStore {
    async fn load(&self) -> anyhow::Result<Vec<MemoryRecord>> {
        if let Some(records) = self.records.read().await.clone() {
            return Ok(records);
        }
        let mut loaded = Vec::new();
        if self.file_path.exists() {
            let contents = fs::read_to_string(&self.file_path).await?;
            for line in contents.lines().filter(|line| !line.trim().is_empty()) {
                loaded.push(serde_json::from_str(line)?);
            }
        }
        *self.records.write().await = Some(loaded.clone());
        Ok(loaded)
    }

    async fn save_all(&self, records: &[MemoryRecord]) -> anyhow::Result<()> {
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let mut out = String::new();
        for record in records {
            out.push_str(&serde_json::to_string(record)?);
            out.push('\n');
        }
        fs::write(&self.file_path, out).await?;
        *self.records.write().await = Some(records.to_vec());
        Ok(())
    }
}

#[async_trait::async_trait]
impl MemoryStore for LocalMemoryStore {
    fn id(&self) -> roder_api::extension::MemoryStoreId {
        "local-memory".to_string()
    }

    async fn put(&self, mut record: MemoryRecord) -> anyhow::Result<MemoryId> {
        let mut records = self.load().await?;
        let id = record
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        record.id = Some(id.clone());
        if let Some(existing) = records
            .iter_mut()
            .find(|existing| existing.id.as_ref() == Some(&id))
        {
            *existing = record;
        } else {
            records.push(record);
        }
        self.save_all(&records).await?;
        Ok(id)
    }

    async fn get(&self, id: &MemoryId) -> anyhow::Result<Option<MemoryRecord>> {
        Ok(self
            .load()
            .await?
            .into_iter()
            .find(|record| record.id.as_ref() == Some(id)))
    }

    async fn search(&self, query: MemoryQuery) -> anyhow::Result<Vec<MemorySearchResult>> {
        let needle = query.text.to_lowercase();
        let mut results = self
            .load()
            .await?
            .into_iter()
            .filter(|record| {
                query
                    .scope
                    .as_ref()
                    .map(|scope| scope == &record.scope)
                    .unwrap_or(true)
            })
            .filter_map(|record| {
                let haystack = record.text.to_lowercase();
                let score = if haystack.contains(&needle) {
                    1.0
                } else {
                    token_overlap(&needle, &haystack)
                };
                (score > 0.0).then_some(MemorySearchResult { record, score })
            })
            .collect::<Vec<_>>();
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(query.limit.max(1));
        Ok(results)
    }

    async fn delete(&self, id: &MemoryId) -> anyhow::Result<()> {
        let mut records = self.load().await?;
        records.retain(|record| record.id.as_ref() != Some(id));
        self.save_all(&records).await
    }
}

pub struct MemoryContextProvider {
    store: Arc<dyn MemoryStore>,
}

#[async_trait::async_trait]
impl ContextProvider for MemoryContextProvider {
    fn id(&self) -> ContextProviderId {
        "memory-context".to_string()
    }

    async fn blocks(&self, query: &ContextQuery) -> anyhow::Result<Vec<ContextBlock>> {
        let results = self
            .store
            .search(MemoryQuery {
                scope: Some(MemoryScope::Workspace(query.thread_id.clone())),
                text: query.prompt.clone(),
                limit: 5,
            })
            .await?;
        Ok(results
            .into_iter()
            .map(|result| ContextBlock {
                id: result
                    .record
                    .id
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                kind: ContextBlockKind::Memory,
                text: result.record.text,
                priority: (result.score * 100.0) as i32,
                token_estimate: None,
                metadata: result.record.metadata,
            })
            .collect())
    }
}

fn token_overlap(needle: &str, haystack: &str) -> f32 {
    let terms = needle.split_whitespace().collect::<Vec<_>>();
    if terms.is_empty() {
        return 0.0;
    }
    let matches = terms
        .iter()
        .filter(|term| haystack.contains(**term))
        .count();
    matches as f32 / terms.len() as f32
}

pub fn extension(base_path: PathBuf) -> MemoryExtension {
    MemoryExtension::new(base_path)
}
