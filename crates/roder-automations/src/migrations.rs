use rusqlite::Connection;

pub const SCHEMA_VERSION: i64 = 1;

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS automations (
            id TEXT PRIMARY KEY,
            definition_json TEXT NOT NULL,
            enabled INTEGER NOT NULL,
            project_cwd TEXT NOT NULL,
            last_checked_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS automation_occurrences (
            occurrence_key TEXT PRIMARY KEY,
            automation_id TEXT NOT NULL,
            scheduled_for INTEGER NOT NULL,
            state TEXT NOT NULL,
            skip_reason TEXT,
            created_at INTEGER NOT NULL,
            FOREIGN KEY (automation_id) REFERENCES automations(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS automation_runs (
            run_id TEXT PRIMARY KEY,
            automation_id TEXT NOT NULL,
            occurrence_key TEXT NOT NULL,
            state TEXT NOT NULL,
            scheduled_for INTEGER NOT NULL,
            summary_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY (automation_id) REFERENCES automations(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS automation_leases (
            run_id TEXT PRIMARY KEY,
            automation_id TEXT NOT NULL,
            occurrence_key TEXT NOT NULL UNIQUE,
            server_id TEXT NOT NULL,
            server_role TEXT NOT NULL,
            leased_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            FOREIGN KEY (automation_id) REFERENCES automations(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS automation_run_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT NOT NULL,
            stream TEXT NOT NULL,
            chunk TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            FOREIGN KEY (run_id) REFERENCES automation_runs(run_id) ON DELETE CASCADE
        );

        PRAGMA user_version = 1;
        "#,
    )
}
