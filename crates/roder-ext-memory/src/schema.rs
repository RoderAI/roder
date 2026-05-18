use rusqlite::Connection;

pub const SCHEMA_VERSION: i64 = 1;

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS memory_schema (
          version INTEGER NOT NULL PRIMARY KEY,
          applied_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS memory_scopes (
          id TEXT PRIMARY KEY,
          kind TEXT NOT NULL,
          value TEXT,
          label TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS memories (
          id TEXT PRIMARY KEY,
          scope_id TEXT NOT NULL,
          text TEXT NOT NULL,
          content_hash TEXT NOT NULL,
          metadata TEXT NOT NULL DEFAULT '{}',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          deleted_at TEXT,
          UNIQUE(scope_id, content_hash, deleted_at),
          FOREIGN KEY(scope_id) REFERENCES memory_scopes(id)
        );

        CREATE TABLE IF NOT EXISTS memory_embeddings (
          memory_id TEXT NOT NULL,
          provider_id TEXT NOT NULL,
          model TEXT NOT NULL,
          dimensions INTEGER NOT NULL,
          embedding BLOB NOT NULL,
          updated_at TEXT NOT NULL,
          PRIMARY KEY(memory_id, provider_id, model),
          FOREIGN KEY(memory_id) REFERENCES memories(id)
        );

        CREATE TABLE IF NOT EXISTS memory_usage (
          memory_id TEXT NOT NULL,
          scope_id TEXT NOT NULL,
          use_count INTEGER NOT NULL DEFAULT 0,
          last_used_at TEXT,
          PRIMARY KEY(memory_id, scope_id),
          FOREIGN KEY(memory_id) REFERENCES memories(id)
        );

        CREATE TABLE IF NOT EXISTS memory_jobs (
          id TEXT PRIMARY KEY,
          kind TEXT NOT NULL,
          scope_id TEXT,
          provider_id TEXT NOT NULL,
          model TEXT NOT NULL,
          status TEXT NOT NULL,
          attempts INTEGER NOT NULL DEFAULT 0,
          leased_until TEXT,
          error TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        "#,
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO memory_schema(version, applied_at) VALUES (?1, datetime('now'))",
        [SCHEMA_VERSION],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_migrates_memory_tables() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'memory_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count >= 5);
    }
}
