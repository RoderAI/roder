use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use roder_api::extension::MemoryStoreId;
use roder_api::memory::{
    MemoryCitation, MemoryId, MemoryQuery, MemoryRecord, MemoryScope, MemorySearchResult,
    MemoryStore, MemoryStoreFactory, MemoryUsageMetadata,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::schema;
use crate::scopes;
use crate::vector;

const DEFAULT_PROVIDER: &str = "fake";
const DEFAULT_MODEL: &str = "fake-vector-32";

pub struct SqliteMemoryStoreFactory {
    base_path: PathBuf,
}

impl SqliteMemoryStoreFactory {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }
}

impl MemoryStoreFactory for SqliteMemoryStoreFactory {
    fn id(&self) -> MemoryStoreId {
        "sqlite-memory".to_string()
    }

    fn create(&self) -> Arc<dyn MemoryStore> {
        Arc::new(SqliteMemoryStore::open(self.base_path.join("memories.sqlite3")).unwrap())
    }
}

pub struct SqliteMemoryStore {
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl SqliteMemoryStore {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
        schema::migrate(&conn)?;
        let store = Self {
            path,
            conn: Mutex::new(conn),
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
            self.put_blocking(record)?;
        }
        std::fs::write(marker, b"imported\n")?;
        Ok(())
    }

    fn put_blocking(&self, mut record: MemoryRecord) -> anyhow::Result<MemoryId> {
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
            let embedding = vector::fake_embedding(&record.text);
            conn.execute(
                "INSERT OR REPLACE INTO memory_embeddings(memory_id, provider_id, model, dimensions, embedding, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    DEFAULT_PROVIDER,
                    DEFAULT_MODEL,
                    embedding.len() as i64,
                    vector::encode(&embedding),
                    format_time(now),
                ],
            )?;
            record.id = Some(id.clone());
            Ok(id)
        })
    }
}

#[async_trait::async_trait]
impl MemoryStore for SqliteMemoryStore {
    fn id(&self) -> MemoryStoreId {
        "sqlite-memory".to_string()
    }

    async fn put(&self, record: MemoryRecord) -> anyhow::Result<MemoryId> {
        self.put_blocking(record)
    }

    async fn get(&self, id: &MemoryId) -> anyhow::Result<Option<MemoryRecord>> {
        self.with_conn(|conn| load_record(conn, id))
    }

    async fn search(&self, query: MemoryQuery) -> anyhow::Result<Vec<MemorySearchResult>> {
        self.with_conn(|conn| search_records(conn, query))
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
        MemoryScope::Session(value) => ("session", Some(value.as_str())),
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
) -> anyhow::Result<Vec<MemorySearchResult>> {
    let mut records = list_records(conn, None, 1000)?;
    if let Some(scope) = &query.scope {
        let scope_id = scope.stable_id();
        records.retain(|record| {
            record.scope.stable_id() == scope_id
                || (query.include_global && record.scope == MemoryScope::Global)
        });
    }
    let query_vector = vector::fake_embedding(&query.text);
    let mut results = records
        .into_iter()
        .filter_map(|record| {
            let score = if query.text.trim().is_empty() {
                1.0
            } else {
                load_embedding(conn, record.id.as_deref()?)
                    .map(|embedding| vector::cosine(&query_vector, &embedding))
                    .unwrap_or_else(|_| lexical_score(&query.text, &record.text))
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

fn load_embedding(conn: &Connection, id: &str) -> anyhow::Result<Vec<f32>> {
    let bytes: Vec<u8> = conn.query_row(
        "SELECT embedding FROM memory_embeddings WHERE memory_id = ?1 AND provider_id = ?2 AND model = ?3",
        params![id, DEFAULT_PROVIDER, DEFAULT_MODEL],
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
        "session" => MemoryScope::Session(value.unwrap_or_default()),
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
}
