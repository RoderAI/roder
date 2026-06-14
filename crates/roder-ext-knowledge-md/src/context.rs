//! Prompt-time knowledge recall: injects relevant project knowledge into
//! turns as `ContextBlockKind::Knowledge` blocks with citations.

use std::path::Path;
use std::sync::Arc;

use roder_api::context::{
    ContextBlock, ContextBlockKind, ContextProvider, ContextProviderId, ContextQuery,
};
use roder_api::knowledge::{KnowledgeQuery, KnowledgeStore};
use roder_api::memory::MemoryScope;

/// Cap on the text a single knowledge document may inject per turn. Bodies
/// are unbounded markdown; recall injects the relevant snippet and points
/// the model at `knowledge_read` for the rest.
const MAX_BLOCK_TEXT_BYTES: usize = 2048;

pub struct KnowledgeContextProvider {
    store: Arc<dyn KnowledgeStore>,
    recall_limit: usize,
}

impl KnowledgeContextProvider {
    pub fn new(store: Arc<dyn KnowledgeStore>) -> Self {
        Self {
            store,
            recall_limit: 4,
        }
    }

    pub fn with_recall_limit(mut self, recall_limit: usize) -> Self {
        self.recall_limit = recall_limit.max(1);
        self
    }
}

#[async_trait::async_trait]
impl ContextProvider for KnowledgeContextProvider {
    fn id(&self) -> ContextProviderId {
        "knowledge-context".to_string()
    }

    async fn blocks(&self, query: &ContextQuery) -> anyhow::Result<Vec<ContextBlock>> {
        let search = self
            .store
            .search(KnowledgeQuery {
                scope: Some(project_scope(query.workspace.as_deref())),
                text: query.prompt.clone(),
                kind: None,
                limit: self.recall_limit,
                include_global: true,
            })
            .await;
        // Knowledge recall is an enrichment; a degraded store must not fail
        // context assembly (and with it the whole turn).
        let results = match search {
            Ok(results) => results,
            Err(error) => {
                eprintln!(
                    "knowledge-context: search failed, continuing without knowledge: {error:#}"
                );
                return Ok(Vec::new());
            }
        };
        Ok(results
            .into_iter()
            .map(|result| ContextBlock {
                id: result.document.id.clone(),
                kind: ContextBlockKind::Knowledge,
                text: block_text(
                    &result.document.title,
                    result.document.kind.as_str(),
                    &result.document.id,
                    &result.snippet,
                ),
                priority: (result.score * 100.0) as i32,
                token_estimate: None,
                metadata: serde_json::json!({
                    "scope": result.document.scope.stable_id(),
                    "citation": result.citation,
                    "kind": result.document.kind,
                }),
            })
            .collect())
    }
}

/// Resolves the project scope from the turn workspace path (its directory
/// name), matching the project key the knowledge tools default to.
fn project_scope(workspace: Option<&str>) -> MemoryScope {
    let from_workspace = workspace.and_then(|workspace| {
        Path::new(workspace)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
    });
    MemoryScope::Project(from_workspace.unwrap_or_else(crate::tools::default_project_key))
}

fn block_text(title: &str, kind: &str, id: &str, snippet: &str) -> String {
    let mut text = format!("Project knowledge ({kind}) \"{title}\" [{id}]: {snippet}");
    if text.len() > MAX_BLOCK_TEXT_BYTES {
        let mut end = MAX_BLOCK_TEXT_BYTES;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("...");
    }
    text.push_str(&format!(
        "\n(Read the full document with knowledge_read id={id}.)"
    ));
    text
}

#[cfg(test)]
mod tests {
    use roder_api::extension::KnowledgeStoreId;
    use roder_api::knowledge::{
        KnowledgeCitation, KnowledgeDocId, KnowledgeDocSummary, KnowledgeDocument, KnowledgeKind,
        KnowledgeLinkRequest, KnowledgeListQuery, KnowledgeRevisionInfo, KnowledgeSaveRequest,
        KnowledgeSearchResult, KnowledgeSource, KnowledgeStatus, KnowledgeUpdateRequest,
    };
    use time::OffsetDateTime;

    use super::*;

    struct StubStore {
        results: anyhow::Result<Vec<KnowledgeSearchResult>>,
    }

    #[async_trait::async_trait]
    impl KnowledgeStore for StubStore {
        fn id(&self) -> KnowledgeStoreId {
            "stub".to_string()
        }
        async fn save(&self, _: KnowledgeSaveRequest) -> anyhow::Result<KnowledgeDocument> {
            unimplemented!()
        }
        async fn get(&self, _: &KnowledgeDocId) -> anyhow::Result<Option<KnowledgeDocument>> {
            Ok(None)
        }
        async fn get_revision(
            &self,
            _: &KnowledgeDocId,
            _: u32,
        ) -> anyhow::Result<Option<KnowledgeDocument>> {
            Ok(None)
        }
        async fn list(&self, _: KnowledgeListQuery) -> anyhow::Result<Vec<KnowledgeDocSummary>> {
            Ok(Vec::new())
        }
        async fn search(&self, _: KnowledgeQuery) -> anyhow::Result<Vec<KnowledgeSearchResult>> {
            match &self.results {
                Ok(results) => Ok(results.clone()),
                Err(error) => Err(anyhow::anyhow!("{error}")),
            }
        }
        async fn update(&self, _: KnowledgeUpdateRequest) -> anyhow::Result<KnowledgeDocument> {
            unimplemented!()
        }
        async fn archive(&self, _: &KnowledgeDocId) -> anyhow::Result<bool> {
            Ok(false)
        }
        async fn set_link(&self, _: KnowledgeLinkRequest) -> anyhow::Result<KnowledgeDocument> {
            unimplemented!()
        }
        async fn revisions(
            &self,
            _: &KnowledgeDocId,
        ) -> anyhow::Result<Vec<KnowledgeRevisionInfo>> {
            Ok(Vec::new())
        }
    }

    fn context_query() -> ContextQuery {
        ContextQuery {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            prompt: "auth requirements".to_string(),
            workspace: Some("/tmp/demo-project".to_string()),
            token_budget: None,
        }
    }

    fn search_result(snippet: String) -> KnowledgeSearchResult {
        let now = OffsetDateTime::UNIX_EPOCH;
        let summary = KnowledgeDocSummary {
            id: "kn-1".to_string(),
            scope: MemoryScope::Project("demo-project".to_string()),
            kind: KnowledgeKind::Requirement,
            slug: "auth".to_string(),
            title: "Auth".to_string(),
            status: KnowledgeStatus::Active,
            source: KnowledgeSource::User,
            tags: Vec::new(),
            links: Vec::new(),
            revision: 1,
            byte_count: snippet.len() as u64,
            preview: snippet.clone(),
            created_at: now,
            updated_at: now,
        };
        KnowledgeSearchResult {
            citation: KnowledgeCitation {
                doc_id: summary.id.clone(),
                scope_id: summary.scope.stable_id(),
                title: summary.title.clone(),
                snippet: snippet.clone(),
                score_millis: 1000,
            },
            document: summary,
            score: 1.0,
            snippet,
        }
    }

    #[test]
    fn provider_id_is_stable() {
        let store = StubStore {
            results: Ok(Vec::new()),
        };
        assert_eq!(
            KnowledgeContextProvider::new(Arc::new(store)).id(),
            "knowledge-context"
        );
    }

    #[tokio::test]
    async fn store_errors_degrade_to_no_blocks() {
        let store = StubStore {
            results: Err(anyhow::anyhow!("disk gone")),
        };
        let provider = KnowledgeContextProvider::new(Arc::new(store));

        let blocks = provider.blocks(&context_query()).await.unwrap();

        assert!(blocks.is_empty());
    }

    #[tokio::test]
    async fn blocks_use_knowledge_kind_with_citation_and_bounded_text() {
        let store = StubStore {
            results: Ok(vec![
                search_result("é".repeat(MAX_BLOCK_TEXT_BYTES)),
                search_result("short snippet".to_string()),
            ]),
        };
        let provider = KnowledgeContextProvider::new(Arc::new(store));

        let blocks = provider.blocks(&context_query()).await.unwrap();

        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0].kind, ContextBlockKind::Knowledge));
        assert!(blocks[0].text.contains("knowledge_read id=kn-1"));
        assert!(blocks[0].text.len() < MAX_BLOCK_TEXT_BYTES + 128);
        assert!(blocks[0].metadata.get("citation").is_some());
        assert!(blocks[1].text.contains("short snippet"));
    }

    #[test]
    fn project_scope_prefers_workspace_directory_name() {
        assert_eq!(
            project_scope(Some("/tmp/demo-project")),
            MemoryScope::Project("demo-project".to_string())
        );
    }
}
