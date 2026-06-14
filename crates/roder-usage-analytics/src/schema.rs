//! SQLite schema and migrations for the local usage-analytics store.

use rusqlite::Connection;

/// Ordered migrations; version = index + 1. Applied versions are recorded
/// in `analytics_schema_migrations` and never re-run.
const MIGRATIONS: &[&str] = &[
    // v1: base schema.
    r#"
    CREATE TABLE sessions (
        thread_id TEXT PRIMARY KEY,
        workspace_key TEXT,
        workspace_label TEXT,
        provider TEXT,
        model TEXT,
        created_at_ms INTEGER NOT NULL,
        updated_at_ms INTEGER NOT NULL
    );
    CREATE INDEX idx_sessions_workspace ON sessions(workspace_key);

    CREATE TABLE turns (
        thread_id TEXT NOT NULL,
        turn_id TEXT NOT NULL,
        provider TEXT,
        model TEXT,
        runtime_profile TEXT,
        started_at_ms INTEGER,
        completed_at_ms INTEGER,
        status TEXT NOT NULL,
        error_kind TEXT,
        PRIMARY KEY (thread_id, turn_id)
    );
    CREATE INDEX idx_turns_started ON turns(started_at_ms);
    CREATE INDEX idx_turns_provider_model ON turns(provider, model);

    CREATE TABLE token_usage (
        thread_id TEXT NOT NULL,
        turn_id TEXT NOT NULL,
        provider TEXT,
        model TEXT,
        recorded_at_ms INTEGER NOT NULL,
        prompt_tokens INTEGER NOT NULL,
        completion_tokens INTEGER NOT NULL,
        total_tokens INTEGER NOT NULL,
        cached_prompt_tokens INTEGER NOT NULL,
        PRIMARY KEY (thread_id, turn_id)
    );
    CREATE INDEX idx_token_usage_recorded ON token_usage(recorded_at_ms);
    CREATE INDEX idx_token_usage_provider_model ON token_usage(provider, model);

    CREATE TABLE tool_calls (
        thread_id TEXT NOT NULL,
        turn_id TEXT NOT NULL,
        tool_id TEXT NOT NULL,
        tool_name TEXT,
        started_at_ms INTEGER,
        completed_at_ms INTEGER,
        duration_ms INTEGER,
        status TEXT NOT NULL,
        is_error INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (thread_id, turn_id, tool_id)
    );
    CREATE INDEX idx_tool_calls_name ON tool_calls(tool_name);
    CREATE INDEX idx_tool_calls_started ON tool_calls(started_at_ms);
    CREATE INDEX idx_tool_calls_duration ON tool_calls(duration_ms);

    CREATE TABLE ingested_event_offsets (
        source_path TEXT PRIMARY KEY,
        last_line INTEGER NOT NULL,
        source_mtime_ms INTEGER,
        updated_at_ms INTEGER NOT NULL
    );

    CREATE TABLE daily_rollups (
        day TEXT NOT NULL,
        workspace_key TEXT NOT NULL DEFAULT '',
        provider TEXT NOT NULL DEFAULT '',
        model TEXT NOT NULL DEFAULT '',
        tool_name TEXT NOT NULL DEFAULT '',
        call_count INTEGER NOT NULL DEFAULT 0,
        error_count INTEGER NOT NULL DEFAULT 0,
        total_duration_ms INTEGER NOT NULL DEFAULT 0,
        p50_duration_ms INTEGER,
        p95_duration_ms INTEGER,
        p99_duration_ms INTEGER,
        prompt_tokens INTEGER NOT NULL DEFAULT 0,
        completion_tokens INTEGER NOT NULL DEFAULT 0,
        total_tokens INTEGER NOT NULL DEFAULT 0,
        cached_prompt_tokens INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (day, workspace_key, provider, model, tool_name)
    );
    CREATE INDEX idx_daily_rollups_day ON daily_rollups(day);
    "#,
];

pub(crate) fn apply_migrations(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS analytics_schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at_ms INTEGER NOT NULL
        );",
    )?;
    let applied: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM analytics_schema_migrations",
        [],
        |row| row.get(0),
    )?;
    for (index, migration) in MIGRATIONS.iter().enumerate() {
        let version = (index + 1) as i64;
        if version <= applied {
            continue;
        }
        conn.execute_batch(migration)?;
        conn.execute(
            "INSERT INTO analytics_schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
            rusqlite::params![version, crate::store::now_ms()],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_migrations_apply_once_and_reopen_safely() {
        let dir =
            std::env::temp_dir().join(format!("roder-analytics-schema-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("usage.sqlite3");

        let conn = Connection::open(&path).unwrap();
        apply_migrations(&conn).unwrap();
        let version: i64 = conn
            .query_row(
                "SELECT MAX(version) FROM analytics_schema_migrations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        drop(conn);

        // Reopening must not re-run applied migrations.
        let conn = Connection::open(&path).unwrap();
        apply_migrations(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM analytics_schema_migrations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, MIGRATIONS.len() as i64);

        // Required indexes exist.
        for index in [
            "idx_tool_calls_name",
            "idx_tool_calls_duration",
            "idx_turns_started",
            "idx_token_usage_provider_model",
            "idx_daily_rollups_day",
        ] {
            let found: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                    [index],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "missing index {index}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
