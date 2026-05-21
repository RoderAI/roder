use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};
use crate::extension::EmbeddingProviderId;

pub type CodeIndexProviderId = String;
pub type CodeIndexStoreId = String;
pub type CodeIndexGenerationId = String;
pub type CodeIndexQueryId = String;
pub type MerkleHash = String;
pub type ContentHash = String;
pub type ChunkHash = String;
pub type PathHash = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeIndexStatus {
    Disabled,
    Missing,
    Building,
    Chunking,
    Embedding,
    Ready,
    Stale,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeIndexNodeKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSimilarityHash {
    pub algorithm: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMerkleNode {
    pub path: PathBuf,
    pub path_hash: PathHash,
    pub content_hash: MerkleHash,
    pub kind: CodeIndexNodeKind,
    #[serde(default)]
    pub children: Vec<MerkleHash>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMerkleTree {
    pub workspace_root: PathBuf,
    pub root_hash: MerkleHash,
    pub similarity_hash: WorkspaceSimilarityHash,
    pub nodes: Vec<WorkspaceMerkleNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeByteRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeLineRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeChunk {
    pub chunk_hash: ChunkHash,
    pub path: PathBuf,
    pub path_hash: PathHash,
    pub byte_range: CodeByteRange,
    pub line_range: CodeLineRange,
    pub content_hash: ContentHash,
    pub language: Option<String>,
    pub symbol_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChunkEmbedding {
    pub chunk_hash: ChunkHash,
    pub provider: EmbeddingProviderId,
    pub model: String,
    pub dimensions: usize,
    #[serde(default)]
    pub vector: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContentProof {
    pub path_hash: PathHash,
    pub content_hash: ContentHash,
    pub workspace_root_hash: MerkleHash,
    pub generation_id: CodeIndexGenerationId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexStats {
    pub file_count: u64,
    pub chunk_count: u64,
    pub embedded_chunk_count: u64,
    pub cached_embedding_count: u64,
    pub index_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IndexGeneration {
    pub id: CodeIndexGenerationId,
    pub status: CodeIndexStatus,
    pub workspace_root: PathBuf,
    pub root_hash: Option<MerkleHash>,
    pub config_hash: String,
    pub stats: CodeIndexStats,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub updated_at: Option<OffsetDateTime>,
    pub stale_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexSearchRequest {
    pub query_id: CodeIndexQueryId,
    pub query: String,
    pub workspace_root: PathBuf,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexSearchResult {
    pub query_id: CodeIndexQueryId,
    pub chunk: CodeChunk,
    pub score: f32,
    pub proof: ContentProof,
    pub proof_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexSearchResponse {
    pub generation: IndexGeneration,
    pub results: Vec<CodeIndexSearchResult>,
    pub dropped_results: Vec<ProofFilteredDrop>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProofFilteredDrop {
    pub query_id: CodeIndexQueryId,
    pub path_hash: PathHash,
    pub content_hash: ContentHash,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexEventContext {
    pub workspace_root: PathBuf,
    pub generation_id: Option<CodeIndexGenerationId>,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexingStarted {
    pub context: CodeIndexEventContext,
    pub config_hash: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexChunked {
    pub context: CodeIndexEventContext,
    pub file_count: u64,
    pub chunk_count: u64,
    pub changed_chunk_count: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexEmbedded {
    pub context: CodeIndexEventContext,
    pub provider: EmbeddingProviderId,
    pub model: String,
    pub embedded_chunk_count: u64,
    pub cached_embedding_count: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexReady {
    pub generation: IndexGeneration,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexStale {
    pub context: CodeIndexEventContext,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexFailed {
    pub context: CodeIndexEventContext,
    pub error: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexProofFilteredResultDropped {
    pub context: CodeIndexEventContext,
    pub drop: ProofFilteredDrop,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[async_trait::async_trait]
pub trait CodeIndexStore: Send + Sync + 'static {
    fn id(&self) -> CodeIndexStoreId;

    async fn status(&self, workspace_root: PathBuf) -> anyhow::Result<IndexGeneration>;

    async fn search(
        &self,
        request: CodeIndexSearchRequest,
    ) -> anyhow::Result<CodeIndexSearchResponse>;

    async fn read_chunk(
        &self,
        proof: ContentProof,
        byte_range: Option<CodeByteRange>,
    ) -> anyhow::Result<Option<String>>;

    async fn list_proofs(&self, workspace_root: PathBuf) -> anyhow::Result<Vec<ContentProof>>;
}

#[async_trait::async_trait]
pub trait CodeIndexProvider: Send + Sync + 'static {
    fn id(&self) -> CodeIndexProviderId;

    async fn rebuild(&self, workspace_root: PathBuf) -> anyhow::Result<IndexGeneration>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_index_search_result_serializes_without_source_by_default() {
        let generation = generation();
        let response = CodeIndexSearchResponse {
            generation,
            results: vec![CodeIndexSearchResult {
                query_id: "query-1".to_string(),
                chunk: chunk(),
                score: 0.82,
                proof: proof(),
                proof_verified: true,
                snippet: None,
            }],
            dropped_results: vec![ProofFilteredDrop {
                query_id: "query-1".to_string(),
                path_hash: "path-denied".to_string(),
                content_hash: "content-denied".to_string(),
                reason: "content proof missing".to_string(),
            }],
        };

        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["generation"]["status"], "ready");
        assert_eq!(value["results"][0]["proofVerified"], true);
        assert!(value["results"][0].get("snippet").is_none());
        assert_eq!(
            value["droppedResults"][0]["reason"],
            "content proof missing"
        );
    }

    #[test]
    fn code_index_events_round_trip_context_and_proof_drop() {
        let event = CodeIndexProofFilteredResultDropped {
            context: CodeIndexEventContext {
                workspace_root: PathBuf::from("/repo"),
                generation_id: Some("gen-1".to_string()),
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
            },
            drop: ProofFilteredDrop {
                query_id: "query-1".to_string(),
                path_hash: "path-x".to_string(),
                content_hash: "content-x".to_string(),
                reason: "path outside workspace scope".to_string(),
            },
            timestamp: OffsetDateTime::UNIX_EPOCH,
        };

        let json = serde_json::to_string(&event).unwrap();
        let round_trip: CodeIndexProofFilteredResultDropped = serde_json::from_str(&json).unwrap();

        assert_eq!(round_trip.context.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(round_trip.drop.reason, "path outside workspace scope");
    }

    fn generation() -> IndexGeneration {
        IndexGeneration {
            id: "gen-1".to_string(),
            status: CodeIndexStatus::Ready,
            workspace_root: PathBuf::from("/repo"),
            root_hash: Some("root-hash".to_string()),
            config_hash: "config-hash".to_string(),
            stats: CodeIndexStats {
                file_count: 2,
                chunk_count: 4,
                embedded_chunk_count: 4,
                cached_embedding_count: 1,
                index_bytes: 128,
            },
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: Some(OffsetDateTime::UNIX_EPOCH),
            stale_reason: None,
        }
    }

    fn chunk() -> CodeChunk {
        CodeChunk {
            chunk_hash: "chunk-1".to_string(),
            path: PathBuf::from("src/lib.rs"),
            path_hash: "path-1".to_string(),
            byte_range: CodeByteRange { start: 0, end: 42 },
            line_range: CodeLineRange { start: 1, end: 3 },
            content_hash: "content-1".to_string(),
            language: Some("rust".to_string()),
            symbol_hint: Some("CodeIndex".to_string()),
        }
    }

    fn proof() -> ContentProof {
        ContentProof {
            path_hash: "path-1".to_string(),
            content_hash: "content-1".to_string(),
            workspace_root_hash: "root-hash".to_string(),
            generation_id: "gen-1".to_string(),
        }
    }
}
