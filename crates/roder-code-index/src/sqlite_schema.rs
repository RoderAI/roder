use std::path::PathBuf;

use roder_api::code_index::{CodeIndexStats, CodeIndexStatus, IndexGeneration};
use rusqlite::{Connection, OptionalExtension, params};
use time::OffsetDateTime;

pub(crate) fn migrate(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS file_manifest (
            path TEXT PRIMARY KEY NOT NULL,
            path_hash TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            size INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS chunks (
            chunk_hash TEXT PRIMARY KEY NOT NULL,
            path TEXT NOT NULL,
            path_hash TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            start_byte INTEGER NOT NULL,
            end_byte INTEGER NOT NULL,
            start_line INTEGER NOT NULL,
            end_line INTEGER NOT NULL,
            language TEXT,
            symbol_hint TEXT,
            embedding_provider TEXT NOT NULL,
            embedding_model TEXT NOT NULL,
            embedding_dimensions INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS embedding_cache (
            content_hash TEXT PRIMARY KEY NOT NULL,
            vector_json TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            dimensions INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS generations (
            id TEXT PRIMARY KEY NOT NULL,
            status TEXT NOT NULL,
            workspace_root TEXT NOT NULL,
            root_hash TEXT,
            config_hash TEXT NOT NULL,
            file_count INTEGER NOT NULL,
            chunk_count INTEGER NOT NULL,
            embedded_chunk_count INTEGER NOT NULL,
            cached_embedding_count INTEGER NOT NULL,
            index_bytes INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT,
            stale_reason TEXT
        );
        ",
    )?;
    Ok(())
}

pub(crate) fn save_generation(
    conn: &Connection,
    generation: &IndexGeneration,
) -> anyhow::Result<()> {
    conn.execute("DELETE FROM generations", [])?;
    conn.execute(
        "INSERT INTO generations(
            id, status, workspace_root, root_hash, config_hash, file_count, chunk_count,
            embedded_chunk_count, cached_embedding_count, index_bytes, created_at,
            updated_at, stale_reason
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            generation.id,
            status_to_str(&generation.status),
            generation.workspace_root.to_string_lossy(),
            generation.root_hash,
            generation.config_hash,
            generation.stats.file_count as i64,
            generation.stats.chunk_count as i64,
            generation.stats.embedded_chunk_count as i64,
            generation.stats.cached_embedding_count as i64,
            generation.stats.index_bytes as i64,
            format_time(generation.created_at)?,
            generation.updated_at.map(format_time).transpose()?,
            generation.stale_reason,
        ],
    )?;
    Ok(())
}

pub(crate) fn load_generation(conn: &Connection) -> anyhow::Result<Option<IndexGeneration>> {
    conn.query_row(
        "SELECT id, status, workspace_root, root_hash, config_hash, file_count, chunk_count,
                embedded_chunk_count, cached_embedding_count, index_bytes, created_at,
                updated_at, stale_reason
         FROM generations
         ORDER BY created_at DESC
         LIMIT 1",
        [],
        |row| {
            let created_at = parse_time(row.get::<_, String>(10)?)?;
            let updated_at = row
                .get::<_, Option<String>>(11)?
                .map(parse_time)
                .transpose()?;
            Ok(IndexGeneration {
                id: row.get(0)?,
                status: status_from_str(&row.get::<_, String>(1)?),
                workspace_root: PathBuf::from(row.get::<_, String>(2)?),
                root_hash: row.get(3)?,
                config_hash: row.get(4)?,
                stats: CodeIndexStats {
                    file_count: row.get::<_, i64>(5)? as u64,
                    chunk_count: row.get::<_, i64>(6)? as u64,
                    embedded_chunk_count: row.get::<_, i64>(7)? as u64,
                    cached_embedding_count: row.get::<_, i64>(8)? as u64,
                    index_bytes: row.get::<_, i64>(9)? as u64,
                },
                created_at,
                updated_at,
                stale_reason: row.get(12)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn status_to_str(status: &CodeIndexStatus) -> &'static str {
    match status {
        CodeIndexStatus::Disabled => "disabled",
        CodeIndexStatus::Missing => "missing",
        CodeIndexStatus::Building => "building",
        CodeIndexStatus::Chunking => "chunking",
        CodeIndexStatus::Embedding => "embedding",
        CodeIndexStatus::Ready => "ready",
        CodeIndexStatus::Stale => "stale",
        CodeIndexStatus::Failed => "failed",
    }
}

fn status_from_str(status: &str) -> CodeIndexStatus {
    match status {
        "disabled" => CodeIndexStatus::Disabled,
        "building" => CodeIndexStatus::Building,
        "chunking" => CodeIndexStatus::Chunking,
        "embedding" => CodeIndexStatus::Embedding,
        "ready" => CodeIndexStatus::Ready,
        "stale" => CodeIndexStatus::Stale,
        "failed" => CodeIndexStatus::Failed,
        _ => CodeIndexStatus::Missing,
    }
}

fn parse_time(value: String) -> rusqlite::Result<OffsetDateTime> {
    OffsetDateTime::parse(&value, &time::format_description::well_known::Rfc3339).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
    })
}

fn format_time(value: OffsetDateTime) -> anyhow::Result<String> {
    Ok(value.format(&time::format_description::well_known::Rfc3339)?)
}
