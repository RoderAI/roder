use std::sync::Arc;

use roder_api::context::{
    ContextBlock, ContextBlockKind, ContextProvider, ContextProviderId, ContextQuery,
};
use roder_api::memory::{MemoryQuery, MemoryScope, MemoryStore};

/// Cap on the text a single memory record may inject per turn. Record text is
/// model-authored and stores do not bound it at rest, so without a cap one
/// verbose note is re-injected whole into every matching turn.
const MAX_BLOCK_TEXT_BYTES: usize = 4096;

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
        let search = self
            .store
            .search(MemoryQuery {
                scope: Some(MemoryScope::Workspace(query.thread_id.clone())),
                text: query.prompt.clone(),
                limit: 5,
                include_global: true,
                provider_id: None,
                model: None,
            })
            .await;
        // Memory recall is an enrichment; a degraded store must not fail
        // context assembly (and with it the whole turn), so errors degrade
        // to "no memory blocks".
        let results = match search {
            Ok(results) => results,
            Err(error) => {
                eprintln!("memory-context: search failed, continuing without memory: {error:#}");
                return Ok(Vec::new());
            }
        };
        Ok(results
            .into_iter()
            .map(|result| ContextBlock {
                id: result
                    .record
                    .id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                kind: ContextBlockKind::Memory,
                text: bounded_block_text(result.record.text),
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

fn bounded_block_text(text: String) -> String {
    if text.len() <= MAX_BLOCK_TEXT_BYTES {
        return text;
    }
    // Truncate on a UTF-8 char boundary at or below the cap.
    let mut end = MAX_BLOCK_TEXT_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}... [truncated; read the full memory with memory_read]",
        &text[..end]
    )
}

#[cfg(test)]
mod tests {
    use roder_api::memory::{MemoryRecord, MemorySearchResult};
    use time::OffsetDateTime;

    use super::*;

    struct StubStore {
        results: anyhow::Result<Vec<MemorySearchResult>>,
    }

    #[async_trait::async_trait]
    impl MemoryStore for StubStore {
        fn id(&self) -> roder_api::extension::MemoryStoreId {
            "stub".to_string()
        }
        async fn put(&self, _: MemoryRecord) -> anyhow::Result<String> {
            Ok("id".to_string())
        }
        async fn get(&self, _: &String) -> anyhow::Result<Option<MemoryRecord>> {
            Ok(None)
        }
        async fn search(&self, _: MemoryQuery) -> anyhow::Result<Vec<MemorySearchResult>> {
            match &self.results {
                Ok(results) => Ok(results.clone()),
                Err(error) => Err(anyhow::anyhow!("{error}")),
            }
        }
        async fn delete(&self, _: &String) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn context_query() -> ContextQuery {
        ContextQuery {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            prompt: "prompt".to_string(),
            workspace: None,
            token_budget: None,
        }
    }

    fn search_result(text: String) -> MemorySearchResult {
        MemorySearchResult {
            record: MemoryRecord {
                id: Some("session/msg".to_string()),
                scope: MemoryScope::Global,
                text,
                content_hash: None,
                metadata: serde_json::Value::Null,
                usage: None,
                deleted: false,
                created_at: OffsetDateTime::UNIX_EPOCH,
                updated_at: OffsetDateTime::UNIX_EPOCH,
            },
            score: 1.0,
            citation: None,
        }
    }

    #[test]
    fn recall_context_provider_id_is_stable() {
        let store = StubStore {
            results: Ok(Vec::new()),
        };
        assert_eq!(
            MemoryContextProvider::new(Arc::new(store)).id(),
            "memory-context"
        );
    }

    #[tokio::test]
    async fn store_errors_degrade_to_no_blocks() {
        let store = StubStore {
            results: Err(anyhow::anyhow!("honcho unreachable")),
        };
        let provider = MemoryContextProvider::new(Arc::new(store));

        let blocks = provider.blocks(&context_query()).await.unwrap();

        assert!(blocks.is_empty());
    }

    #[tokio::test]
    async fn block_text_is_capped_per_record() {
        let store = StubStore {
            results: Ok(vec![
                search_result("é".repeat(MAX_BLOCK_TEXT_BYTES)),
                search_result("short note".to_string()),
            ]),
        };
        let provider = MemoryContextProvider::new(Arc::new(store));

        let blocks = provider.blocks(&context_query()).await.unwrap();

        assert!(
            blocks[0]
                .text
                .ends_with("[truncated; read the full memory with memory_read]")
        );
        assert!(blocks[0].text.len() < MAX_BLOCK_TEXT_BYTES + 64);
        // Truncation never splits the 2-byte char; the result stays valid UTF-8.
        assert!(blocks[0].text.starts_with('é'));
        assert_eq!(blocks[1].text, "short note");
    }
}
