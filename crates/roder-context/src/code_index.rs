use std::path::{Path, PathBuf};
use std::sync::Arc;

use roder_api::code_index::{CodeIndexSearchRequest, CodeIndexStats, CodeIndexStatus};
use roder_api::context::{
    ContextBlock, ContextBlockKind, ContextProvider, ContextProviderId, ContextQuery,
};
use roder_code_index::sqlite::SqliteCodeIndexStore;
use serde::Serialize;
use serde_json::json;

const MAX_RESULTS: usize = 5;
const MAX_BLOCK_BYTES: usize = 4 * 1024;
const MAX_SNIPPET_BYTES: usize = 480;

#[derive(Clone)]
pub struct CodeIndexContextProvider {
    workspace: PathBuf,
    store: Arc<SqliteCodeIndexStore>,
}

impl CodeIndexContextProvider {
    pub fn new(workspace: impl Into<PathBuf>, store: Arc<SqliteCodeIndexStore>) -> Self {
        Self {
            workspace: workspace.into(),
            store,
        }
    }
}

#[async_trait::async_trait]
impl ContextProvider for CodeIndexContextProvider {
    fn id(&self) -> ContextProviderId {
        "code-index-context-provider".to_string()
    }

    async fn blocks(&self, query: &ContextQuery) -> anyhow::Result<Vec<ContextBlock>> {
        if query
            .workspace
            .as_deref()
            .is_some_and(|workspace| Path::new(workspace) != self.workspace)
        {
            return Ok(Vec::new());
        }
        let status = self.store.status(&self.workspace)?;
        if status.status != CodeIndexStatus::Ready {
            return Ok(Vec::new());
        }

        let response = self.store.search(CodeIndexSearchRequest {
            query_id: format!("{}:{}", query.thread_id, query.turn_id),
            query: query.prompt.clone(),
            workspace_root: self.workspace.clone(),
            limit: MAX_RESULTS,
        })?;
        if response.results.is_empty() {
            return Ok(Vec::new());
        }

        let mut rows = Vec::new();
        for result in response.results {
            let snippet = bounded_snippet(&self.workspace, &result.chunk).unwrap_or_default();
            rows.push(RenderedCodeResult {
                path: result.chunk.path.to_string_lossy().replace('\\', "/"),
                start_line: result.chunk.line_range.start,
                end_line: result.chunk.line_range.end,
                score: result.score,
                chunk_hash: result.chunk.chunk_hash,
                proof_verified: result.proof_verified,
                snippet,
            });
        }

        let text = render_block_text(&rows);
        let text = truncate_block(text);
        Ok(vec![ContextBlock {
            id: "code-index-context-provider".to_string(),
            kind: ContextBlockKind::RetrievedDocument,
            text,
            priority: 86,
            token_estimate: None,
            metadata: json!({
                "provider": "code-index-context-provider",
                "source": "indexed_semantic_code_search",
                "query": query.prompt,
                "result_count": rows.len(),
                "proof_filtered_drop_count": response.dropped_results.len(),
                "stale_index_fallback": false,
                "index_status": "ready",
                "generation_id": response.generation.id,
                "root_hash": response.generation.root_hash,
                "stats": stats_metadata(&response.generation.stats),
                "results": rows,
            }),
        }])
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct RenderedCodeResult {
    path: String,
    start_line: u32,
    end_line: u32,
    score: f32,
    chunk_hash: String,
    proof_verified: bool,
    snippet: String,
}

fn render_block_text(results: &[RenderedCodeResult]) -> String {
    let mut text = String::from("Indexed semantic code context:");
    for result in results {
        text.push_str(&format!(
            "\n- {}:{}-{} score {:.2} proof {} chunk {}",
            result.path,
            result.start_line,
            result.end_line,
            result.score,
            if result.proof_verified {
                "verified"
            } else {
                "unverified"
            },
            &result.chunk_hash[..12.min(result.chunk_hash.len())]
        ));
        if !result.snippet.is_empty() {
            text.push_str("\n  ```\n");
            text.push_str(&result.snippet);
            if !result.snippet.ends_with('\n') {
                text.push('\n');
            }
            text.push_str("  ```");
        }
    }
    text
}

fn bounded_snippet(
    workspace: &Path,
    chunk: &roder_api::code_index::CodeChunk,
) -> anyhow::Result<String> {
    let workspace = std::fs::canonicalize(workspace)?;
    let path = std::fs::canonicalize(workspace.join(&chunk.path))?;
    if !path.starts_with(&workspace) {
        return Ok(String::new());
    }
    let text = std::fs::read_to_string(path)?;
    let start = chunk.byte_range.start as usize;
    let end = chunk
        .byte_range
        .end
        .min(start as u64 + MAX_SNIPPET_BYTES as u64) as usize;
    if start >= text.len() || end > text.len() || start >= end {
        return Ok(String::new());
    }
    let mut start = start;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    let mut end = end;
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(text[start..end].to_string())
}

fn truncate_block(mut text: String) -> String {
    if text.len() <= MAX_BLOCK_BYTES {
        return text;
    }
    text.truncate(MAX_BLOCK_BYTES);
    while !text.is_char_boundary(text.len()) {
        text.pop();
    }
    text.push_str("\n...");
    text
}

fn stats_metadata(stats: &CodeIndexStats) -> serde_json::Value {
    json!({
        "fileCount": stats.file_count,
        "chunkCount": stats.chunk_count,
        "embeddedChunkCount": stats.embedded_chunk_count,
        "cachedEmbeddingCount": stats.cached_embedding_count,
        "indexBytes": stats.index_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::context::ContextQuery;

    #[tokio::test]
    async fn code_index_provider_returns_bounded_verified_snippets() {
        let root = test_workspace("provider-bounded-snippets");
        write(
            &root,
            "src/auth.rs",
            "pub fn oauth_refresh_token() {\n    let token = \"refresh\";\n}\n",
        );
        let store = Arc::new(SqliteCodeIndexStore::open(root.with_extension("sqlite3")).unwrap());
        store.rebuild_workspace(&root).unwrap();
        let provider = CodeIndexContextProvider::new(root.clone(), store);

        let blocks = provider
            .blocks(&query("oauth refresh token"))
            .await
            .unwrap();

        assert_eq!(blocks.len(), 1);
        let block = &blocks[0];
        assert_eq!(block.kind, ContextBlockKind::RetrievedDocument);
        assert!(block.text.contains("src/auth.rs:"));
        assert!(block.text.contains("proof verified"));
        assert!(block.text.len() <= MAX_BLOCK_BYTES + 4);
        assert_eq!(block.metadata["source"], "indexed_semantic_code_search");
        assert!(block.metadata["result_count"].as_u64().unwrap() >= 1);
        assert_eq!(block.metadata["results"][0]["proofVerified"], true);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn code_index_provider_degrades_to_empty_when_index_missing() {
        let root = test_workspace("provider-missing-index");
        let store = Arc::new(SqliteCodeIndexStore::open(root.with_extension("sqlite3")).unwrap());
        let provider = CodeIndexContextProvider::new(root.clone(), store);

        let blocks = provider.blocks(&query("anything")).await.unwrap();

        assert!(blocks.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    fn query(prompt: &str) -> ContextQuery {
        ContextQuery {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            prompt: prompt.to_string(),
            workspace: None,
            token_budget: None,
        }
    }

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn test_workspace(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("roder-code-context-{name}-{stamp}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
