use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context;
use roder_api::code_index::{
    ChunkEmbedding, CodeChunk, CodeIndexSearchRequest, CodeIndexSearchResponse,
    CodeIndexSearchResult, CodeIndexStats, CodeIndexStatus, IndexGeneration, ProofFilteredDrop,
};
use rusqlite::{Connection, params};
use time::OffsetDateTime;

use crate::chunk::chunk_workspace;
use crate::merkle::{FileManifestEntry, build_workspace_merkle, diff_file_manifests};
use crate::proofs::{proof_for_chunk, verify_chunk_proof};
use crate::sqlite_embeddings::ensure_embedding;
use crate::sqlite_schema::{load_generation, migrate, save_generation};

const STORE_ID: &str = "sqlite-code-index";
const CONFIG_HASH: &str = "local-code-index-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildStats {
    pub generation: IndexGeneration,
    pub changed_file_count: u64,
    pub deleted_file_count: u64,
    pub reused_file_count: u64,
}

#[derive(Debug, Clone)]
pub struct StoredChunk {
    pub chunk: CodeChunk,
    pub embedding: ChunkEmbedding,
}

pub struct SqliteCodeIndexStore {
    path: PathBuf,
    conn: Mutex<Connection>,
}

pub fn default_store_path(base_dir: impl AsRef<Path>, workspace_root: impl AsRef<Path>) -> PathBuf {
    base_dir
        .as_ref()
        .join(workspace_key(workspace_root.as_ref()))
        .join("code-index.sqlite3")
}

impl SqliteCodeIndexStore {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("open code index sqlite store {}", path.display()))?;
        migrate(&conn)?;
        Ok(Self {
            path,
            conn: Mutex::new(conn),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn id(&self) -> &'static str {
        STORE_ID
    }

    pub fn rebuild_workspace(
        &self,
        workspace_root: impl AsRef<Path>,
    ) -> anyhow::Result<RebuildStats> {
        let build = build_workspace_merkle(workspace_root.as_ref())?;
        let chunks = chunk_workspace(&build.tree.workspace_root, &build.files)?;
        self.with_conn(|conn| {
            let previous = load_file_manifest(conn)?;
            let diff = diff_file_manifests(&previous, &build.files);
            let generation_id = generation_id(&build.tree.root_hash);
            let mut embedded_chunk_count = 0u64;
            let mut cached_embedding_count = 0u64;

            let tx = conn.unchecked_transaction()?;
            tx.execute("DELETE FROM chunks", [])?;
            tx.execute("DELETE FROM file_manifest", [])?;

            for file in &build.files {
                tx.execute(
                    "INSERT INTO file_manifest(path, path_hash, content_hash, size)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        path_to_string(&file.path),
                        file.path_hash,
                        file.content_hash,
                        file.size as i64
                    ],
                )?;
            }

            for chunk in &chunks {
                let (embedding, cached) = ensure_embedding(&tx, chunk)?;
                if cached {
                    cached_embedding_count += 1;
                } else {
                    embedded_chunk_count += 1;
                }
                tx.execute(
                    "INSERT INTO chunks(
                        chunk_hash, path, path_hash, content_hash, start_byte, end_byte,
                        start_line, end_line, language, symbol_hint, embedding_provider,
                        embedding_model, embedding_dimensions
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        chunk.chunk_hash,
                        path_to_string(&chunk.path),
                        chunk.path_hash,
                        chunk.content_hash,
                        chunk.byte_range.start as i64,
                        chunk.byte_range.end as i64,
                        chunk.line_range.start as i64,
                        chunk.line_range.end as i64,
                        chunk.language,
                        chunk.symbol_hint,
                        embedding.provider,
                        embedding.model,
                        embedding.dimensions as i64,
                    ],
                )?;
            }

            let generation = IndexGeneration {
                id: generation_id,
                status: CodeIndexStatus::Ready,
                workspace_root: build.tree.workspace_root.clone(),
                root_hash: Some(build.tree.root_hash.clone()),
                config_hash: CONFIG_HASH.to_string(),
                stats: CodeIndexStats {
                    file_count: build.files.len() as u64,
                    chunk_count: chunks.len() as u64,
                    embedded_chunk_count,
                    cached_embedding_count,
                    index_bytes: index_bytes(&tx)?,
                },
                created_at: OffsetDateTime::now_utc(),
                updated_at: Some(OffsetDateTime::now_utc()),
                stale_reason: None,
            };
            save_generation(&tx, &generation)?;
            tx.commit()?;

            Ok(RebuildStats {
                generation,
                changed_file_count: diff.changed_files.len() as u64,
                deleted_file_count: diff.deleted_files.len() as u64,
                reused_file_count: diff.unchanged_files.len() as u64,
            })
        })
    }

    pub fn status(&self, workspace_root: impl AsRef<Path>) -> anyhow::Result<IndexGeneration> {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        self.with_conn(|conn| {
            load_generation(conn)?.map_or_else(
                || {
                    Ok(IndexGeneration {
                        id: "missing".to_string(),
                        status: CodeIndexStatus::Missing,
                        workspace_root,
                        root_hash: None,
                        config_hash: CONFIG_HASH.to_string(),
                        stats: CodeIndexStats {
                            file_count: 0,
                            chunk_count: 0,
                            embedded_chunk_count: 0,
                            cached_embedding_count: 0,
                            index_bytes: 0,
                        },
                        created_at: OffsetDateTime::now_utc(),
                        updated_at: None,
                        stale_reason: Some("code index has not been built".to_string()),
                    })
                },
                Ok,
            )
        })
    }

    pub fn list_chunks(&self) -> anyhow::Result<Vec<StoredChunk>> {
        self.with_conn(load_chunks)
    }

    pub fn search(
        &self,
        request: CodeIndexSearchRequest,
    ) -> anyhow::Result<CodeIndexSearchResponse> {
        let query_terms = tokenize(&request.query);
        self.with_conn(|conn| {
            let generation = load_generation(conn)?
                .with_context(|| "code index search requested before generation exists")?;
            let root_hash = generation.root_hash.clone().unwrap_or_default();
            let mut scored = Vec::new();
            let mut dropped_results = Vec::new();
            for stored in load_chunks(conn)? {
                let score = score_chunk(&stored.chunk, &query_terms);
                if score <= 0.0 {
                    continue;
                }
                let proof = proof_for_chunk(&root_hash, generation.id.clone(), &stored.chunk);
                if !verify_chunk_proof(&proof, &root_hash, &stored.chunk) {
                    dropped_results.push(ProofFilteredDrop {
                        query_id: request.query_id.clone(),
                        path_hash: stored.chunk.path_hash,
                        content_hash: stored.chunk.content_hash,
                        reason: "content proof failed".to_string(),
                    });
                    continue;
                }
                scored.push(CodeIndexSearchResult {
                    query_id: request.query_id.clone(),
                    chunk: stored.chunk,
                    score,
                    proof,
                    proof_verified: true,
                    snippet: None,
                });
            }
            scored.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.chunk.path.cmp(&b.chunk.path))
            });
            scored.truncate(request.limit);

            Ok(CodeIndexSearchResponse {
                generation,
                results: scored,
                dropped_results,
            })
        })
    }

    fn with_conn<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("code index sqlite connection lock poisoned"))?;
        f(&mut conn)
    }
}

fn load_file_manifest(conn: &Connection) -> anyhow::Result<Vec<FileManifestEntry>> {
    let mut stmt = conn
        .prepare("SELECT path, path_hash, content_hash, size FROM file_manifest ORDER BY path")?;
    let rows = stmt.query_map([], |row| {
        Ok(FileManifestEntry {
            path: PathBuf::from(row.get::<_, String>(0)?),
            path_hash: row.get(1)?,
            content_hash: row.get(2)?,
            size: row.get::<_, i64>(3)? as u64,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_chunks(conn: &mut Connection) -> anyhow::Result<Vec<StoredChunk>> {
    let mut stmt = conn.prepare(
        "SELECT c.chunk_hash, c.path, c.path_hash, c.content_hash, c.start_byte, c.end_byte,
                c.start_line, c.end_line, c.language, c.symbol_hint,
                e.vector_json, e.provider, e.model, e.dimensions
         FROM chunks c
         JOIN embedding_cache e ON e.content_hash = c.content_hash
         ORDER BY c.path, c.start_byte",
    )?;
    let rows = stmt.query_map([], |row| {
        let vector_json: String = row.get(10)?;
        let vector: Vec<f32> = serde_json::from_str(&vector_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                10,
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        })?;
        let chunk = CodeChunk {
            chunk_hash: row.get(0)?,
            path: PathBuf::from(row.get::<_, String>(1)?),
            path_hash: row.get(2)?,
            content_hash: row.get(3)?,
            byte_range: roder_api::code_index::CodeByteRange {
                start: row.get::<_, i64>(4)? as u64,
                end: row.get::<_, i64>(5)? as u64,
            },
            line_range: roder_api::code_index::CodeLineRange {
                start: row.get::<_, i64>(6)? as u32,
                end: row.get::<_, i64>(7)? as u32,
            },
            language: row.get(8)?,
            symbol_hint: row.get(9)?,
        };
        let embedding = ChunkEmbedding {
            chunk_hash: chunk.chunk_hash.clone(),
            provider: row.get(11)?,
            model: row.get(12)?,
            dimensions: row.get::<_, i64>(13)? as usize,
            vector,
        };
        Ok(StoredChunk { chunk, embedding })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn index_bytes(conn: &Connection) -> anyhow::Result<u64> {
    let page_count: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
    let page_size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    Ok((page_count * page_size).max(0) as u64)
}

fn generation_id(root_hash: &str) -> String {
    format!("gen-{}", &root_hash[..16.min(root_hash.len())])
}

fn workspace_key(workspace_root: &Path) -> String {
    crate::hex_sha256(workspace_root.to_string_lossy().as_bytes())
}

fn path_to_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn tokenize(query: &str) -> BTreeSet<String> {
    query
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .filter(|term| !term.is_empty())
        .map(|term| term.to_ascii_lowercase())
        .collect()
}

fn score_chunk(chunk: &CodeChunk, terms: &BTreeSet<String>) -> f32 {
    if terms.is_empty() {
        return 0.0;
    }
    let mut haystack = path_to_string(&chunk.path).to_ascii_lowercase();
    if let Some(symbol) = &chunk.symbol_hint {
        haystack.push(' ');
        haystack.push_str(&symbol.to_ascii_lowercase());
    }
    let matches = terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .count();
    matches as f32 / terms.len() as f32
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn sqlite_rebuild_caches_unchanged_chunks_and_tracks_file_changes() {
        let root = tempdir("sqlite_rebuild_caches_unchanged_chunks_and_tracks_file_changes");
        write(root.join("src/a.rs"), "pub fn a() {}\n");
        write(root.join("src/b.rs"), "pub fn b() {}\n");
        let store = SqliteCodeIndexStore::open(root.with_extension("sqlite3")).unwrap();

        let first = store.rebuild_workspace(&root).unwrap();
        assert_eq!(first.generation.stats.embedded_chunk_count, 2);
        assert_eq!(first.generation.stats.cached_embedding_count, 0);

        write(root.join("src/a.rs"), "pub fn a_changed() {}\n");
        let second = store.rebuild_workspace(&root).unwrap();

        assert_eq!(second.changed_file_count, 1);
        assert_eq!(second.reused_file_count, 1);
        assert_eq!(second.generation.stats.embedded_chunk_count, 1);
        assert_eq!(second.generation.stats.cached_embedding_count, 1);
    }

    #[test]
    fn sqlite_rebuild_removes_deleted_files_from_chunks_and_results() {
        let root = tempdir("sqlite_rebuild_removes_deleted_files_from_chunks_and_results");
        write(root.join("src/keep.rs"), "pub fn keep_token() {}\n");
        write(root.join("src/delete.rs"), "pub fn delete_token() {}\n");
        let store = SqliteCodeIndexStore::open(root.with_extension("sqlite3")).unwrap();
        store.rebuild_workspace(&root).unwrap();

        fs::remove_file(root.join("src/delete.rs")).unwrap();
        let second = store.rebuild_workspace(&root).unwrap();
        assert_eq!(second.deleted_file_count, 1);

        let chunks = store.list_chunks().unwrap();
        assert!(
            chunks
                .iter()
                .all(|stored| stored.chunk.path != PathBuf::from("src/delete.rs"))
        );

        let response = store
            .search(CodeIndexSearchRequest {
                query_id: "q1".to_string(),
                query: "delete_token".to_string(),
                workspace_root: root.clone(),
                limit: 10,
            })
            .unwrap();
        assert!(response.results.is_empty());
    }

    fn write(path: PathBuf, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn tempdir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "roder-code-index-{name}-{}-{}",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
