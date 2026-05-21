use std::path::{Path, PathBuf};

use roder_api::code_index::{CodeIndexSearchRequest, CodeIndexStatus, ContentProof};
use roder_code_index::merkle::build_workspace_merkle;
use roder_code_index::proofs::proof_for_chunk;
use roder_code_index::sqlite::{SqliteCodeIndexStore, default_store_path};
use roder_protocol::{
    CodeIndexChunkReadPage, CodeIndexProofsListParams, CodeIndexProofsListResult,
    CodeIndexReadChunkParams, CodeIndexReadChunkResult, CodeIndexRebuildParams,
    CodeIndexRebuildResult, CodeIndexSearchParams, CodeIndexSearchResultEnvelope,
    CodeIndexStatusNotification, CodeIndexStatusParams, CodeIndexStatusResult, CodeIndexStatusView,
    JsonRpcError, JsonRpcNotification,
};

use crate::server::AppServer;

const READ_CHUNK_MAX_BYTES: usize = 4096;

impl AppServer {
    pub async fn handle_code_index_status(
        &self,
        params: CodeIndexStatusParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self.code_index_workspace(params.workspace.as_deref());
        let store = self.code_index_store(&workspace)?;
        Ok(serde_json::to_value(CodeIndexStatusResult {
            status: self.code_index_status_for(&workspace, &store)?,
        })
        .unwrap())
    }

    pub async fn handle_code_index_rebuild(
        &self,
        params: CodeIndexRebuildParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self.code_index_workspace(params.workspace.as_deref());
        let store = self.code_index_store(&workspace)?;
        store
            .rebuild_workspace(&workspace)
            .map_err(internal_error)?;
        let status = self.code_index_status_for(&workspace, &store)?;
        self.publish_code_index_status(status.clone());
        Ok(serde_json::to_value(CodeIndexRebuildResult { status }).unwrap())
    }

    pub async fn handle_code_index_search(
        &self,
        params: CodeIndexSearchParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if params.query.trim().is_empty() {
            return Err(invalid_params("index/search requires query"));
        }
        let workspace = self.code_index_workspace(params.workspace.as_deref());
        let store = self.code_index_store(&workspace)?;
        let status = self.code_index_status_for(&workspace, &store)?;
        let mut response = store
            .search(CodeIndexSearchRequest {
                query_id: uuid::Uuid::new_v4().to_string(),
                query: params.query,
                workspace_root: workspace.clone(),
                limit: params.limit.unwrap_or(5).clamp(1, 50),
            })
            .map_err(internal_error)?;
        if status.stale {
            response.generation.status = CodeIndexStatus::Stale;
            response.generation.stale_reason = status.message.clone();
        }
        Ok(serde_json::to_value(CodeIndexSearchResultEnvelope { status, response }).unwrap())
    }

    pub async fn handle_code_index_read_chunk(
        &self,
        params: CodeIndexReadChunkParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if !params.include_source {
            return Err(invalid_params(
                "index/readChunk requires includeSource=true for policy-gated source reads",
            ));
        }
        let workspace = self.code_index_workspace(params.workspace.as_deref());
        let store = self.code_index_store(&workspace)?;
        let status = self.code_index_status_for(&workspace, &store)?;
        let chunk = store
            .list_chunks()
            .map_err(internal_error)?
            .into_iter()
            .find(|stored| stored.chunk.chunk_hash == params.chunk_hash)
            .map(|stored| stored.chunk)
            .ok_or_else(|| invalid_params("unknown code index chunk hash"))?;
        let source = read_scoped_chunk_source(&workspace, &chunk).map_err(internal_error)?;
        let offset = previous_char_boundary(&source, params.offset.unwrap_or(0).min(source.len()));
        let limit = params
            .limit
            .unwrap_or(READ_CHUNK_MAX_BYTES)
            .clamp(1, READ_CHUNK_MAX_BYTES);
        let end = previous_char_boundary(&source, (offset + limit).min(source.len()));
        let text = source[offset..end].to_string();
        let page = CodeIndexChunkReadPage {
            chunk,
            text,
            offset,
            limit,
            total_bytes: source.len(),
            has_more: end < source.len(),
        };
        Ok(serde_json::to_value(CodeIndexReadChunkResult { status, page }).unwrap())
    }

    pub async fn handle_code_index_proofs_list(
        &self,
        params: CodeIndexProofsListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self.code_index_workspace(params.workspace.as_deref());
        let store = self.code_index_store(&workspace)?;
        let status = self.code_index_status_for(&workspace, &store)?;
        let root_hash = status.root_hash.clone().unwrap_or_default();
        let generation_id = status.generation_id.clone().unwrap_or_default();
        let proofs = store
            .list_chunks()
            .map_err(internal_error)?
            .into_iter()
            .map(|stored| proof_for_chunk(&root_hash, generation_id.clone(), &stored.chunk))
            .collect::<Vec<ContentProof>>();
        Ok(serde_json::to_value(CodeIndexProofsListResult { status, proofs }).unwrap())
    }

    fn code_index_status_for(
        &self,
        workspace: &Path,
        store: &SqliteCodeIndexStore,
    ) -> Result<CodeIndexStatusView, JsonRpcError> {
        let generation = store.status(workspace).map_err(internal_error)?;
        let stale = generation.status == CodeIndexStatus::Ready
            && generation
                .root_hash
                .as_ref()
                .is_some_and(|root_hash| root_hash != &current_root_hash(workspace));
        let status = if stale {
            CodeIndexStatus::Stale
        } else {
            generation.status.clone()
        };
        Ok(CodeIndexStatusView {
            status,
            workspace: workspace.display().to_string(),
            store_path: store.path().display().to_string(),
            generation_id: (generation.id != "missing").then_some(generation.id),
            root_hash: generation.root_hash,
            stale,
            stats: generation.stats,
            message: if stale {
                Some("code index is stale; rebuild to refresh semantic search".to_string())
            } else {
                generation.stale_reason
            },
        })
    }

    fn code_index_store(&self, workspace: &Path) -> Result<SqliteCodeIndexStore, JsonRpcError> {
        SqliteCodeIndexStore::open(self.code_index_store_path(workspace)).map_err(internal_error)
    }

    fn code_index_workspace(&self, workspace: Option<&str>) -> PathBuf {
        workspace
            .map(PathBuf::from)
            .unwrap_or_else(|| self.runtime.workspace())
    }

    fn code_index_store_path(&self, workspace: &Path) -> PathBuf {
        let base = std::env::var_os("RODER_CODE_INDEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join(".roder")
                    .join("code-index")
            });
        default_store_path(base, workspace)
    }

    fn publish_code_index_status(&self, status: CodeIndexStatusView) {
        self.publish_notification(JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "index/statusChanged".to_string(),
            params: serde_json::to_value(CodeIndexStatusNotification { status }).unwrap(),
        });
    }
}

fn current_root_hash(workspace: &Path) -> String {
    build_workspace_merkle(workspace)
        .map(|build| build.tree.root_hash)
        .unwrap_or_default()
}

fn read_scoped_chunk_source(
    workspace: &Path,
    chunk: &roder_api::code_index::CodeChunk,
) -> anyhow::Result<String> {
    let workspace = std::fs::canonicalize(workspace)?;
    let path = std::fs::canonicalize(workspace.join(&chunk.path))?;
    if !path.starts_with(&workspace) {
        anyhow::bail!("chunk path escapes workspace");
    }
    let text = std::fs::read_to_string(path)?;
    let start = chunk.byte_range.start as usize;
    let end = chunk.byte_range.end as usize;
    if start > end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        anyhow::bail!("chunk byte range is invalid for current file content");
    }
    Ok(text[start..end].to_string())
}

fn previous_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {err}"),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_index_read_pagination_snaps_to_utf8_boundaries() {
        let text = "αβγ token";
        let offset = previous_char_boundary(text, 1);
        let end = previous_char_boundary(text, 5);

        assert_eq!(offset, 0);
        assert_eq!(&text[offset..end], "αβ");
    }
}
