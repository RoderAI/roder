use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use roder_api::embeddings::EmbeddingProvider;
use roder_api::extension::MemoryStoreId;
use roder_api::memory::{
    MemoryCitation, MemoryId, MemoryQuery, MemoryRecord, MemoryScope, MemorySearchResult,
    MemoryStore, MemoryStoreFactory, MemoryUsageMetadata,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::embed::{FALLBACK_PROVIDER, MemoryEmbedder, MemoryEmbedding};
use crate::schema;
use crate::scopes;
use crate::vector;

pub struct SqliteMemoryStoreFactory {
    base_path: PathBuf,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl SqliteMemoryStoreFactory {
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            base_path,
            embedding_provider: None,
        }
    }

    pub fn with_embedding_provider(
        mut self,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Self {
        self.embedding_provider = embedding_provider;
        self
    }
}

impl MemoryStoreFactory for SqliteMemoryStoreFactory {
    fn id(&self) -> MemoryStoreId {
        "sqlite-memory".to_string()
    }

    fn create(&self) -> Arc<dyn MemoryStore> {
        Arc::new(
            SqliteMemoryStore::open_with_embedding_provider(
                self.base_path.join("memories.sqlite3"),
                self.embedding_provider.clone(),
            )
            .unwrap(),
        )
    }
}

pub struct SqliteMemoryStore {
    path: PathBuf,
    conn: Mutex<Connection>,
    embedder: MemoryEmbedder,
}

impl SqliteMemoryStore {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        Self::open_with_embedding_provider(path, None)
    }

    pub fn open_with_embedding_provider(
        path: PathBuf,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
        schema::migrate(&conn)?;
        let store = Self {
            path,
            conn: Mutex::new(conn),
            embedder: MemoryEmbedder::new(embedding_provider),
        };
        store.import_jsonl_once()?;
        Ok(store)
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("memory sqlite connection lock poisoned"))?;
        f(&conn)
    }

    fn import_jsonl_once(&self) -> anyhow::Result<()> {
        let Some(dir) = self.path.parent() else {
            return Ok(());
        };
        let jsonl = dir.join("memories.jsonl");
        let marker = dir.join(".memories-jsonl-imported");
        if !jsonl.exists() || marker.exists() {
            return Ok(());
        }
        let contents = std::fs::read_to_string(&jsonl)?;
        for line in contents.lines().filter(|line| !line.trim().is_empty()) {
            let record: MemoryRecord = serde_json::from_str(line)?;
            let embedding = MemoryEmbedder::fallback_embedding(&record.text);
            self.put_blocking(record, embedding)?;
        }
        std::fs::write(marker, b"imported\n")?;
        Ok(())
    }

    fn put_blocking(
        &self,
        mut record: MemoryRecord,
        embedding: MemoryEmbedding,
    ) -> anyhow::Result<MemoryId> {
        self.with_conn(|conn| {
            let now = OffsetDateTime::now_utc();
            let id = record
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let hash = record
                .content_hash
                .clone()
                .unwrap_or_else(|| content_hash(&record.text));
            let scope_id = ensure_scope(conn, &record.scope)?;
            let metadata = serde_json::to_string(&record.metadata)?;
            conn.execute(
                "INSERT INTO memories(id, scope_id, text, content_hash, metadata, created_at, updated_at, deleted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)
                 ON CONFLICT(id) DO UPDATE SET text = excluded.text, content_hash = excluded.content_hash,
                   metadata = excluded.metadata, updated_at = excluded.updated_at, deleted_at = NULL",
                params![
                    id,
                    scope_id,
                    record.text,
                    hash,
                    metadata,
                    format_time(record.created_at),
                    format_time(now),
                ],
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO memory_embeddings(memory_id, provider_id, model, dimensions, embedding, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    embedding.provider_id,
                    embedding.model,
                    embedding.values.len() as i64,
                    vector::encode(&embedding.values),
                    format_time(now),
                ],
            )?;
            record.id = Some(id.clone());
            Ok(id)
        })
    }

    async fn query_embedding(&self, query: &MemoryQuery) -> Option<MemoryEmbedding> {
        if query.text.trim().is_empty() {
            return None;
        }
        if let Some(provider_id) = query.provider_id.as_deref() {
            if provider_id == FALLBACK_PROVIDER {
                return Some(MemoryEmbedder::fallback_embedding(&query.text));
            }
            if !self.embedder.can_embed_provider(provider_id) {
                return None;
            }
        }
        Some(
            self.embedder
                .embed(&query.text, query.model.as_deref())
                .await,
        )
    }
}

#[async_trait::async_trait]
impl MemoryStore for SqliteMemoryStore {
    fn id(&self) -> MemoryStoreId {
        "sqlite-memory".to_string()
    }

    async fn put(&self, record: MemoryRecord) -> anyhow::Result<MemoryId> {
        let embedding = self.embedder.embed(&record.text, None).await;
        self.put_blocking(record, embedding)
    }

    async fn get(&self, id: &MemoryId) -> anyhow::Result<Option<MemoryRecord>> {
        self.with_conn(|conn| load_record(conn, id))
    }

    async fn search(&self, query: MemoryQuery) -> anyhow::Result<Vec<MemorySearchResult>> {
        let query_embedding = self.query_embedding(&query).await;
        self.with_conn(|conn| search_records(conn, query, query_embedding.as_ref()))
    }

    async fn delete(&self, id: &MemoryId) -> anyhow::Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE memories SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
                params![format_time(OffsetDateTime::now_utc()), id],
            )?;
            Ok(())
        })
    }

    async fn list(
        &self,
        scope: Option<MemoryScope>,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryRecord>> {
        self.with_conn(|conn| list_records(conn, scope, limit))
    }
}

fn ensure_scope(conn: &Connection, scope: &MemoryScope) -> anyhow::Result<String> {
    let desc = scopes::descriptor(scope.clone());
    let (kind, value) = match scope {
        MemoryScope::Global => ("global", None),
        MemoryScope::User(value) => ("user", Some(value.as_str())),
        MemoryScope::Workspace(value) => ("workspace", Some(value.as_str())),
        MemoryScope::Project(value) => ("project", Some(value.as_str())),
        MemoryScope::Thread(value) => ("thread", Some(value.as_str())),
    };
    conn.execute(
        "INSERT OR IGNORE INTO memory_scopes(id, kind, value, label, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            desc.id,
            kind,
            value,
            desc.label,
            format_time(OffsetDateTime::now_utc())
        ],
    )?;
    Ok(scope.stable_id())
}

fn list_records(
    conn: &Connection,
    scope: Option<MemoryScope>,
    limit: usize,
) -> anyhow::Result<Vec<MemoryRecord>> {
    let limit = limit.clamp(1, 200) as i64;
    let mut records = Vec::new();
    if let Some(scope) = scope {
        let scope_id = scope.stable_id();
        let mut stmt = conn.prepare(
            "SELECT m.id, s.kind, s.value, m.text, m.content_hash, m.metadata, m.created_at, m.updated_at,
                    m.deleted_at IS NOT NULL, COALESCE(u.use_count, 0), u.last_used_at
             FROM memories m
             JOIN memory_scopes s ON s.id = m.scope_id
             LEFT JOIN memory_usage u ON u.memory_id = m.id AND u.scope_id = m.scope_id
             WHERE m.scope_id = ?1 AND m.deleted_at IS NULL
             ORDER BY m.updated_at DESC
             LIMIT ?2",
        )?;
        for row in stmt.query_map(params![scope_id, limit], row_to_record)? {
            records.push(row?);
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT m.id, s.kind, s.value, m.text, m.content_hash, m.metadata, m.created_at, m.updated_at,
                    m.deleted_at IS NOT NULL, COALESCE(u.use_count, 0), u.last_used_at
             FROM memories m
             JOIN memory_scopes s ON s.id = m.scope_id
             LEFT JOIN memory_usage u ON u.memory_id = m.id AND u.scope_id = m.scope_id
             WHERE m.deleted_at IS NULL
             ORDER BY m.updated_at DESC
             LIMIT ?1",
        )?;
        for row in stmt.query_map(params![limit], row_to_record)? {
            records.push(row?);
        }
    }
    Ok(records)
}

fn load_record(conn: &Connection, id: &str) -> anyhow::Result<Option<MemoryRecord>> {
    conn.query_row(
        "SELECT m.id, s.kind, s.value, m.text, m.content_hash, m.metadata, m.created_at, m.updated_at,
                m.deleted_at IS NOT NULL, COALESCE(u.use_count, 0), u.last_used_at
         FROM memories m
         JOIN memory_scopes s ON s.id = m.scope_id
         LEFT JOIN memory_usage u ON u.memory_id = m.id AND u.scope_id = m.scope_id
         WHERE m.id = ?1",
        params![id],
        row_to_record,
    )
    .optional()
    .map_err(Into::into)
}

fn search_records(
    conn: &Connection,
    query: MemoryQuery,
    query_embedding: Option<&MemoryEmbedding>,
) -> anyhow::Result<Vec<MemorySearchResult>> {
    let mut records = list_records(conn, None, 1000)?;
    if let Some(scope) = &query.scope {
        let scope_id = scope.stable_id();
        records.retain(|record| {
            record.scope.stable_id() == scope_id
                || (query.include_global && record.scope == MemoryScope::Global)
        });
    }
    let mut results = records
        .into_iter()
        .filter_map(|record| {
            let score = if query.text.trim().is_empty() {
                1.0
            } else if let Some(query_embedding) = query_embedding {
                load_embedding(
                    conn,
                    record.id.as_deref()?,
                    &query_embedding.provider_id,
                    &query_embedding.model,
                )
                .map(|embedding| vector::cosine(&query_embedding.values, &embedding))
                .unwrap_or_else(|_| lexical_score(&query.text, &record.text))
            } else {
                lexical_score(&query.text, &record.text)
            };
            (score > 0.0 || query.text.trim().is_empty()).then(|| {
                let memory_id = record.id.clone().unwrap_or_default();
                MemorySearchResult {
                    citation: Some(MemoryCitation {
                        memory_id,
                        scope_id: record.scope.stable_id(),
                        snippet: snippet(&record.text),
                        score_millis: (score.max(0.0) * 1000.0) as u32,
                    }),
                    record,
                    score,
                }
            })
        })
        .collect::<Vec<_>>();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(query.limit.max(1));
    for result in &results {
        if let Some(id) = &result.record.id {
            mark_used(conn, id, &result.record.scope)?;
        }
    }
    Ok(results)
}

fn load_embedding(
    conn: &Connection,
    id: &str,
    provider_id: &str,
    model: &str,
) -> anyhow::Result<Vec<f32>> {
    let bytes: Vec<u8> = conn.query_row(
        "SELECT embedding FROM memory_embeddings WHERE memory_id = ?1 AND provider_id = ?2 AND model = ?3",
        params![id, provider_id, model],
        |row| row.get(0),
    )?;
    Ok(vector::decode(&bytes))
}

fn mark_used(conn: &Connection, id: &str, scope: &MemoryScope) -> anyhow::Result<()> {
    let scope_id = scope.stable_id();
    conn.execute(
        "INSERT INTO memory_usage(memory_id, scope_id, use_count, last_used_at)
         VALUES (?1, ?2, 1, ?3)
         ON CONFLICT(memory_id, scope_id) DO UPDATE SET use_count = use_count + 1, last_used_at = excluded.last_used_at",
        params![id, scope_id, format_time(OffsetDateTime::now_utc())],
    )?;
    Ok(())
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
    let id: String = row.get(0)?;
    let kind: String = row.get(1)?;
    let value: Option<String> = row.get(2)?;
    let text: String = row.get(3)?;
    let content_hash: String = row.get(4)?;
    let metadata: String = row.get(5)?;
    let created_at: String = row.get(6)?;
    let updated_at: String = row.get(7)?;
    let deleted: bool = row.get(8)?;
    let use_count: i64 = row.get(9)?;
    let last_used_at: Option<String> = row.get(10)?;
    Ok(MemoryRecord {
        id: Some(id),
        scope: parse_scope(&kind, value),
        text,
        content_hash: Some(content_hash),
        metadata: serde_json::from_str(&metadata).unwrap_or(Value::Null),
        usage: Some(MemoryUsageMetadata {
            use_count: use_count.max(0) as u64,
            last_used_at: last_used_at.and_then(|ts| parse_time(&ts).ok()),
        }),
        deleted,
        created_at: parse_time(&created_at).unwrap_or(OffsetDateTime::UNIX_EPOCH),
        updated_at: parse_time(&updated_at).unwrap_or(OffsetDateTime::UNIX_EPOCH),
    })
}

fn parse_scope(kind: &str, value: Option<String>) -> MemoryScope {
    match kind {
        "global" => MemoryScope::Global,
        "user" => MemoryScope::User(value.unwrap_or_default()),
        "workspace" => MemoryScope::Workspace(value.unwrap_or_default()),
        "project" => MemoryScope::Project(value.unwrap_or_default()),
        "thread" => MemoryScope::Thread(value.unwrap_or_default()),
        _ => MemoryScope::Workspace(value.unwrap_or_else(|| "unknown".to_string())),
    }
}

fn lexical_score(query: &str, text: &str) -> f32 {
    let query = query.to_lowercase();
    let text = text.to_lowercase();
    let terms = query.split_whitespace().collect::<Vec<_>>();
    if terms.is_empty() {
        return 1.0;
    }
    terms.iter().filter(|term| text.contains(**term)).count() as f32 / terms.len() as f32
}

fn snippet(text: &str) -> String {
    const MAX: usize = 180;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        let mut out = text.chars().take(MAX).collect::<String>();
        out.push_str("...");
        out
    }
}

pub fn content_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn format_time(time: OffsetDateTime) -> String {
    time.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| OffsetDateTime::UNIX_EPOCH.to_string())
}

fn parse_time(input: &str) -> anyhow::Result<OffsetDateTime> {
    Ok(OffsetDateTime::parse(
        input,
        &time::format_description::well_known::Rfc3339,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::embeddings::{
        EmbeddingModelDescriptor, EmbeddingProvider, EmbeddingProviderDescriptor, EmbeddingRequest,
        EmbeddingResponse, EmbeddingVector,
    };

    fn record(text: &str, scope: MemoryScope) -> MemoryRecord {
        MemoryRecord {
            id: None,
            scope,
            text: text.to_string(),
            content_hash: None,
            metadata: serde_json::json!({}),
            usage: None,
            deleted: false,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    #[derive(Clone)]
    struct TestEmbeddingProvider;

    #[async_trait::async_trait]
    impl EmbeddingProvider for TestEmbeddingProvider {
        fn descriptor(&self) -> EmbeddingProviderDescriptor {
            EmbeddingProviderDescriptor {
                id: "test".to_string(),
                name: "Test Embeddings".to_string(),
                default_model: "test-model".to_string(),
                models: vec![EmbeddingModelDescriptor {
                    id: "test-model".to_string(),
                    dimensions: 3,
                    default: true,
                }],
            }
        }

        async fn embed(&self, request: EmbeddingRequest) -> anyhow::Result<EmbeddingResponse> {
            let embeddings = request
                .inputs
                .into_iter()
                .enumerate()
                .map(|(index, input)| EmbeddingVector {
                    index,
                    values: if input.contains("needle") {
                        vec![1.0, 0.0, 0.0]
                    } else {
                        vec![0.0, 1.0, 0.0]
                    },
                })
                .collect();
            Ok(EmbeddingResponse {
                provider_id: "test".to_string(),
                model: request.model,
                embeddings,
            })
        }
    }

    #[tokio::test]
    async fn sqlite_store_persists_and_searches_project_memory() {
        let path =
            std::env::temp_dir().join(format!("roder-memory-{}.sqlite3", uuid::Uuid::new_v4()));
        let store = SqliteMemoryStore::open(path).unwrap();
        let id = store
            .put(record(
                "sqlite vector memory",
                MemoryScope::Project("p".to_string()),
            ))
            .await
            .unwrap();
        assert!(store.get(&id).await.unwrap().is_some());
        let results = store
            .search(MemoryQuery {
                scope: Some(MemoryScope::Project("p".to_string())),
                text: "sqlite memory".to_string(),
                limit: 5,
                include_global: false,
                provider_id: None,
                model: None,
            })
            .await
            .unwrap();
        assert_eq!(results[0].record.id.as_deref(), Some(id.as_str()));
    }

    #[tokio::test]
    async fn sqlite_store_uses_configured_embedding_provider() {
        let path = std::env::temp_dir().join(format!(
            "roder-memory-provider-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        let store = SqliteMemoryStore::open_with_embedding_provider(
            path,
            Some(Arc::new(TestEmbeddingProvider)),
        )
        .unwrap();
        let wanted = store
            .put(record(
                "needle vector memory",
                MemoryScope::Project("p".to_string()),
            ))
            .await
            .unwrap();
        store
            .put(record(
                "billing policy",
                MemoryScope::Project("p".to_string()),
            ))
            .await
            .unwrap();

        let row = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT provider_id, model, dimensions FROM memory_embeddings WHERE memory_id = ?1",
                    params![&wanted],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(row, ("test".to_string(), "test-model".to_string(), 3));

        let results = store
            .search(MemoryQuery {
                scope: Some(MemoryScope::Project("p".to_string())),
                text: "needle".to_string(),
                limit: 5,
                include_global: false,
                provider_id: None,
                model: None,
            })
            .await
            .unwrap();
        assert_eq!(results[0].record.id.as_deref(), Some(wanted.as_str()));
    }
}
