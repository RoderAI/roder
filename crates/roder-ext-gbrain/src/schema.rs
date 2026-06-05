//! SQLite schema for the bi-temporal gbrain store.

use rusqlite::Connection;

pub const SCHEMA_VERSION: i64 = 1;

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS gbrain_schema (
          version INTEGER NOT NULL PRIMARY KEY,
          applied_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS gbrain_scopes (
          id TEXT PRIMARY KEY,
          kind TEXT NOT NULL,
          value TEXT,
          label TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        -- Facts are invalidated/retracted, never hard-deleted.
        CREATE TABLE IF NOT EXISTS gbrain_facts (
          id TEXT PRIMARY KEY,
          scope_id TEXT NOT NULL,
          subject TEXT,
          text TEXT NOT NULL,
          content_hash TEXT NOT NULL,
          metadata TEXT NOT NULL DEFAULT '{}',
          valid_at TEXT NOT NULL,
          invalid_at TEXT,
          ingested_at TEXT NOT NULL,
          expired_at TEXT,
          supersedes TEXT,
          superseded_by TEXT,
          supersession_reason TEXT,
          provenance TEXT NOT NULL DEFAULT '[]',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id)
        );

        CREATE INDEX IF NOT EXISTS idx_gbrain_facts_scope ON gbrain_facts(scope_id);
        CREATE INDEX IF NOT EXISTS idx_gbrain_facts_subject ON gbrain_facts(scope_id, subject);
        CREATE INDEX IF NOT EXISTS idx_gbrain_facts_valid ON gbrain_facts(valid_at);
        CREATE INDEX IF NOT EXISTS idx_gbrain_facts_ingested ON gbrain_facts(ingested_at);

        CREATE TABLE IF NOT EXISTS gbrain_embeddings (
          fact_id TEXT NOT NULL,
          provider_id TEXT NOT NULL,
          model TEXT NOT NULL,
          dimensions INTEGER NOT NULL,
          embedding BLOB NOT NULL,
          updated_at TEXT NOT NULL,
          PRIMARY KEY(fact_id, provider_id, model),
          FOREIGN KEY(fact_id) REFERENCES gbrain_facts(id)
        );

        -- The supersession / contradiction graph.
        -- kind = 'supersedes' (from_id supersedes to_id) | 'contradicts' (undirected, stored canonically from<to).
        CREATE TABLE IF NOT EXISTS gbrain_links (
          from_id TEXT NOT NULL,
          to_id TEXT NOT NULL,
          kind TEXT NOT NULL,
          reason TEXT,
          created_at TEXT NOT NULL,
          PRIMARY KEY(from_id, to_id, kind),
          FOREIGN KEY(from_id) REFERENCES gbrain_facts(id),
          FOREIGN KEY(to_id) REFERENCES gbrain_facts(id)
        );

        CREATE INDEX IF NOT EXISTS idx_gbrain_links_to ON gbrain_links(to_id, kind);
        CREATE INDEX IF NOT EXISTS idx_gbrain_links_from ON gbrain_links(from_id, kind);
        "#,
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO gbrain_schema(version, applied_at) VALUES (?1, datetime('now'))",
        [SCHEMA_VERSION],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_migrates_gbrain_tables() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        // Idempotent.
        migrate(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'gbrain_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count >= 5, "expected >=5 gbrain_ tables, got {count}");
    }
}
