use std::sync::Arc;

use roder_api::context::{
    ContextBlock, ContextBlockKind, ContextProvider, ContextProviderId, ContextQuery,
};
use roder_api::memory::{MemoryQuery, MemoryScope, MemoryStore};

pub struct MemoryContextProvider {
    store: Arc<dyn MemoryStore>,
}

impl MemoryContextProvider {
    pub fn new(store: Arc<dyn MemoryStore>) -> Self {
        Self { store }
    }
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
                include_global: true,
                provider_id: None,
                model: None,
            })
            .await?;
        Ok(results
            .into_iter()
            .map(|result| ContextBlock {
                id: result
                    .record
                    .id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                kind: ContextBlockKind::Memory,
                text: result.record.text,
                priority: (result.score * 100.0) as i32,
                token_estimate: None,
                metadata: serde_json::json!({
                    "scope": result.record.scope.stable_id(),
                    "citation": result.citation,
                    "usage": result.record.usage,
                }),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recall_context_provider_id_is_stable() {
        struct EmptyStore;
        #[async_trait::async_trait]
        impl MemoryStore for EmptyStore {
            fn id(&self) -> roder_api::extension::MemoryStoreId {
                "empty".to_string()
            }
            async fn put(&self, _: roder_api::memory::MemoryRecord) -> anyhow::Result<String> {
                Ok("id".to_string())
            }
            async fn get(
                &self,
                _: &String,
            ) -> anyhow::Result<Option<roder_api::memory::MemoryRecord>> {
                Ok(None)
            }
            async fn search(
                &self,
                _: MemoryQuery,
            ) -> anyhow::Result<Vec<roder_api::memory::MemorySearchResult>> {
                Ok(vec![])
            }
            async fn delete(&self, _: &String) -> anyhow::Result<()> {
                Ok(())
            }
        }
        assert_eq!(
            MemoryContextProvider::new(Arc::new(EmptyStore)).id(),
            "memory-context"
        );
    }
}
