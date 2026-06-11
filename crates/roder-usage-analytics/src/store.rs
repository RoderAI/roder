//! SQLite-backed analytics store with idempotent upserts.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context;
use rusqlite::{Connection, params};

use crate::model::{
    SessionRecord, TokenUsageRecord, ToolCallRecord, TurnRecord, WorkspaceLabelMode,
};

pub(crate) fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

pub struct AnalyticsStore {
    pub(crate) conn: Mutex<Connection>,
    path: PathBuf,
    pub workspace_label_mode: WorkspaceLabelMode,
}

impl AnalyticsStore {
    /// Opens (creating directories and schema as needed) the analytics
    /// database at `path`.
    pub fn open(path: &Path, workspace_label_mode: WorkspaceLabelMode) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create analytics dir {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("open analytics database {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        crate::schema::apply_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
            workspace_label_mode,
        })
    }

    /// Default location under a Roder data directory.
    pub fn default_path(data_dir: &Path) -> PathBuf {
        data_dir.join("analytics/usage.sqlite3")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Upserts session metadata. Later non-null values win; `created_at_ms`
    /// keeps the earliest observed value.
    pub fn upsert_session(&self, record: &SessionRecord) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (thread_id, workspace_key, workspace_label, provider, model, \
             created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(thread_id) DO UPDATE SET
               workspace_key = COALESCE(excluded.workspace_key, sessions.workspace_key),
               workspace_label = COALESCE(excluded.workspace_label, sessions.workspace_label),
               provider = COALESCE(excluded.provider, sessions.provider),
               model = COALESCE(excluded.model, sessions.model),
               created_at_ms = MIN(sessions.created_at_ms, excluded.created_at_ms),
               updated_at_ms = MAX(sessions.updated_at_ms, excluded.updated_at_ms)",
            params![
                record.thread_id,
                record.workspace_key,
                record.workspace_label,
                record.provider,
                record.model,
                record.created_at_ms,
                record.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    /// Upserts a turn keyed by `(thread_id, turn_id)`. Terminal statuses
    /// (`completed`/`failed`) are never downgraded back to `running`.
    pub fn upsert_turn(&self, record: &TurnRecord) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO turns (thread_id, turn_id, provider, model, runtime_profile, \
             started_at_ms, completed_at_ms, status, error_kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(thread_id, turn_id) DO UPDATE SET
               provider = COALESCE(excluded.provider, turns.provider),
               model = COALESCE(excluded.model, turns.model),
               runtime_profile = COALESCE(excluded.runtime_profile, turns.runtime_profile),
               started_at_ms = COALESCE(turns.started_at_ms, excluded.started_at_ms),
               completed_at_ms = COALESCE(excluded.completed_at_ms, turns.completed_at_ms),
               status = CASE
                 WHEN turns.status IN ('completed', 'failed') AND excluded.status = 'running'
                   THEN turns.status
                 ELSE excluded.status
               END,
               error_kind = COALESCE(excluded.error_kind, turns.error_kind)",
            params![
                record.thread_id,
                record.turn_id,
                record.provider,
                record.model,
                record.runtime_profile,
                record.started_at_ms,
                record.completed_at_ms,
                record.status,
                record.error_kind,
            ],
        )?;
        Ok(())
    }

    /// Upserts terminal token usage for a turn keyed by
    /// `(thread_id, turn_id)`; replaying the same terminal event is a no-op
    /// rather than a double count.
    pub fn upsert_token_usage(&self, record: &TokenUsageRecord) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO token_usage (thread_id, turn_id, provider, model, recorded_at_ms, \
             prompt_tokens, completion_tokens, total_tokens, cached_prompt_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(thread_id, turn_id) DO UPDATE SET
               provider = COALESCE(excluded.provider, token_usage.provider),
               model = COALESCE(excluded.model, token_usage.model),
               recorded_at_ms = excluded.recorded_at_ms,
               prompt_tokens = excluded.prompt_tokens,
               completion_tokens = excluded.completion_tokens,
               total_tokens = excluded.total_tokens,
               cached_prompt_tokens = excluded.cached_prompt_tokens",
            params![
                record.thread_id,
                record.turn_id,
                record.provider,
                record.model,
                record.recorded_at_ms,
                record.prompt_tokens,
                record.completion_tokens,
                record.total_tokens,
                record.cached_prompt_tokens,
            ],
        )?;
        Ok(())
    }

    /// Upserts a tool call keyed by `(thread_id, turn_id, tool_id)`,
    /// merging start/completion halves into one logical record.
    pub fn upsert_tool_call(&self, record: &ToolCallRecord) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tool_calls (thread_id, turn_id, tool_id, tool_name, started_at_ms, \
             completed_at_ms, duration_ms, status, is_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(thread_id, turn_id, tool_id) DO UPDATE SET
               tool_name = COALESCE(excluded.tool_name, tool_calls.tool_name),
               started_at_ms = COALESCE(tool_calls.started_at_ms, excluded.started_at_ms),
               completed_at_ms = COALESCE(excluded.completed_at_ms, tool_calls.completed_at_ms),
               duration_ms = COALESCE(
                 excluded.duration_ms,
                 tool_calls.duration_ms,
                 CASE
                   WHEN excluded.completed_at_ms IS NOT NULL
                        AND tool_calls.started_at_ms IS NOT NULL
                     THEN MAX(0, excluded.completed_at_ms - tool_calls.started_at_ms)
                 END
               ),
               status = CASE
                 WHEN tool_calls.status IN ('success', 'error') AND excluded.status = 'running'
                   THEN tool_calls.status
                 ELSE excluded.status
               END,
               is_error = MAX(tool_calls.is_error, excluded.is_error)",
            params![
                record.thread_id,
                record.turn_id,
                record.tool_id,
                record.tool_name,
                record.started_at_ms,
                record.completed_at_ms,
                record.duration_ms,
                record.status,
                record.is_error,
            ],
        )?;
        Ok(())
    }

    // -- import offsets ---------------------------------------------------

    pub fn import_offset(&self, source_path: &str) -> anyhow::Result<Option<u64>> {
        let conn = self.conn.lock().unwrap();
        let mut statement =
            conn.prepare("SELECT last_line FROM ingested_event_offsets WHERE source_path = ?1")?;
        let mut rows = statement.query([source_path])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get::<_, i64>(0)? as u64)),
            None => Ok(None),
        }
    }

    pub fn record_import_offset(
        &self,
        source_path: &str,
        last_line: u64,
        source_mtime_ms: Option<i64>,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO ingested_event_offsets (source_path, last_line, source_mtime_ms, \
             updated_at_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(source_path) DO UPDATE SET
               last_line = excluded.last_line,
               source_mtime_ms = excluded.source_mtime_ms,
               updated_at_ms = excluded.updated_at_ms",
            params![source_path, last_line as i64, source_mtime_ms, now_ms()],
        )?;
        Ok(())
    }

    /// Clears all analytics rows (used by `--rebuild` before replaying
    /// JSONL). The schema and migrations are kept.
    pub fn clear_all(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "DELETE FROM sessions;
             DELETE FROM turns;
             DELETE FROM token_usage;
             DELETE FROM tool_calls;
             DELETE FROM ingested_event_offsets;
             DELETE FROM daily_rollups;",
        )?;
        Ok(())
    }

    pub fn counts(&self) -> anyhow::Result<StoreCounts> {
        let conn = self.conn.lock().unwrap();
        let count = |table: &str| -> anyhow::Result<u64> {
            Ok(conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })? as u64)
        };
        Ok(StoreCounts {
            sessions: count("sessions")?,
            turns: count("turns")?,
            token_usage: count("token_usage")?,
            tool_calls: count("tool_calls")?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreCounts {
    pub sessions: u64,
    pub turns: u64,
    pub token_usage: u64,
    pub tool_calls: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (AnalyticsStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "roder-analytics-store-{}",
            uuid::Uuid::new_v4()
        ));
        let store = AnalyticsStore::open(
            &AnalyticsStore::default_path(&dir),
            WorkspaceLabelMode::FullPath,
        )
        .unwrap();
        (store, dir)
    }

    #[test]
    fn store_upserts_are_idempotent_and_merge_partial_halves() {
        let (store, dir) = temp_store();

        // Tool start + completion merge into one record with a duration.
        store
            .upsert_tool_call(&ToolCallRecord {
                thread_id: "t1".into(),
                turn_id: "u1".into(),
                tool_id: "call-1".into(),
                tool_name: Some("read_file".into()),
                started_at_ms: Some(1_000),
                completed_at_ms: None,
                duration_ms: None,
                status: "running".into(),
                is_error: false,
            })
            .unwrap();
        store
            .upsert_tool_call(&ToolCallRecord {
                thread_id: "t1".into(),
                turn_id: "u1".into(),
                tool_id: "call-1".into(),
                tool_name: None,
                started_at_ms: None,
                completed_at_ms: Some(1_125),
                duration_ms: None,
                status: "success".into(),
                is_error: false,
            })
            .unwrap();

        // Replaying the completion does not double-count.
        store
            .upsert_tool_call(&ToolCallRecord {
                thread_id: "t1".into(),
                turn_id: "u1".into(),
                tool_id: "call-1".into(),
                tool_name: None,
                started_at_ms: None,
                completed_at_ms: Some(1_125),
                duration_ms: None,
                status: "success".into(),
                is_error: false,
            })
            .unwrap();

        let counts = store.counts().unwrap();
        assert_eq!(counts.tool_calls, 1);
        let (duration, status, name): (i64, String, String) = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT duration_ms, status, tool_name FROM tool_calls",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(duration, 125);
        assert_eq!(status, "success");
        assert_eq!(name, "read_file");

        // Terminal turn status is not downgraded by a late running upsert.
        store
            .upsert_turn(&TurnRecord {
                thread_id: "t1".into(),
                turn_id: "u1".into(),
                provider: Some("mock".into()),
                model: Some("mock".into()),
                runtime_profile: None,
                started_at_ms: Some(900),
                completed_at_ms: Some(2_000),
                status: "completed".into(),
                error_kind: None,
            })
            .unwrap();
        store
            .upsert_turn(&TurnRecord {
                thread_id: "t1".into(),
                turn_id: "u1".into(),
                provider: None,
                model: None,
                runtime_profile: None,
                started_at_ms: Some(900),
                completed_at_ms: None,
                status: "running".into(),
                error_kind: None,
            })
            .unwrap();
        let status: String = store
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT status FROM turns", [], |row| row.get(0))
            .unwrap();
        assert_eq!(status, "completed");

        // Token usage replays update in place.
        for _ in 0..2 {
            store
                .upsert_token_usage(&TokenUsageRecord {
                    thread_id: "t1".into(),
                    turn_id: "u1".into(),
                    provider: Some("mock".into()),
                    model: Some("mock".into()),
                    recorded_at_ms: 2_000,
                    prompt_tokens: 100,
                    completion_tokens: 20,
                    total_tokens: 120,
                    cached_prompt_tokens: 80,
                })
                .unwrap();
        }
        let counts = store.counts().unwrap();
        assert_eq!(counts.token_usage, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_records_no_payload_columns() {
        let (store, dir) = temp_store();
        // The schema itself must not have any column that could hold prompt
        // or output bodies.
        let conn = store.conn.lock().unwrap();
        let mut statement = conn
            .prepare("SELECT name FROM pragma_table_info('tool_calls')")
            .unwrap();
        let columns: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for forbidden in ["output", "arguments", "payload", "prompt", "text"] {
            assert!(
                !columns.iter().any(|column| column.contains(forbidden)),
                "tool_calls must not store {forbidden}"
            );
        }
        drop(statement);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_offsets_round_trip() {
        let (store, dir) = temp_store();
        assert_eq!(store.import_offset("a/events.jsonl").unwrap(), None);
        store
            .record_import_offset("a/events.jsonl", 42, Some(1_000))
            .unwrap();
        assert_eq!(store.import_offset("a/events.jsonl").unwrap(), Some(42));
        store
            .record_import_offset("a/events.jsonl", 99, Some(2_000))
            .unwrap();
        assert_eq!(store.import_offset("a/events.jsonl").unwrap(), Some(99));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
