use std::path::Path;

use anyhow::Result;
use roder_api::automations::{
    AutomationDefinition, AutomationId, AutomationLeaseRecord, AutomationRunId, AutomationRunState,
    AutomationRunSummary,
};
use rusqlite::{Connection, OptionalExtension, params};
use time::OffsetDateTime;

use crate::migrations;
use crate::model::{OccurrenceAction, RunLogEntry, ScheduledOccurrence, StoredAutomation};

pub struct AutomationStore {
    conn: Connection,
}

impl AutomationStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        migrations::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        migrations::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn upsert_automation(
        &self,
        definition: &AutomationDefinition,
        last_checked_at: Option<OffsetDateTime>,
    ) -> Result<()> {
        let definition_json = serde_json::to_string(definition)?;
        self.conn.execute(
            r#"
            INSERT INTO automations (id, definition_json, enabled, project_cwd, last_checked_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(id) DO UPDATE SET
                definition_json = excluded.definition_json,
                enabled = excluded.enabled,
                project_cwd = excluded.project_cwd,
                last_checked_at = excluded.last_checked_at
            "#,
            params![
                definition.id,
                definition_json,
                definition.enabled,
                definition.project.cwd,
                last_checked_at.map(|value| value.unix_timestamp()),
            ],
        )?;
        Ok(())
    }

    pub fn get_automation(&self, id: &str) -> Result<Option<StoredAutomation>> {
        self.conn
            .query_row(
                "SELECT definition_json, last_checked_at FROM automations WHERE id = ?1",
                params![id],
                |row| {
                    let json: String = row.get(0)?;
                    let last_checked_at: Option<i64> = row.get(1)?;
                    Ok((json, last_checked_at))
                },
            )
            .optional()?
            .map(|(json, last_checked_at)| {
                Ok(StoredAutomation {
                    definition: serde_json::from_str(&json)?,
                    last_checked_at: last_checked_at.map(offset_from_unix).transpose()?,
                })
            })
            .transpose()
    }

    pub fn list_automations(&self) -> Result<Vec<StoredAutomation>> {
        let mut statement = self
            .conn
            .prepare("SELECT definition_json, last_checked_at FROM automations ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            let json: String = row.get(0)?;
            let last_checked_at: Option<i64> = row.get(1)?;
            Ok((json, last_checked_at))
        })?;

        rows.map(|row| {
            let (json, last_checked_at) = row?;
            Ok(StoredAutomation {
                definition: serde_json::from_str(&json)?,
                last_checked_at: last_checked_at.map(offset_from_unix).transpose()?,
            })
        })
        .collect()
    }

    pub fn record_occurrence(
        &self,
        occurrence: &ScheduledOccurrence,
        now: OffsetDateTime,
    ) -> Result<()> {
        let (state, skip_reason) = match &occurrence.action {
            OccurrenceAction::Run => ("scheduled", None),
            OccurrenceAction::Skip { reason } => ("skipped", Some(reason.as_str())),
        };
        self.conn.execute(
            r#"
            INSERT INTO automation_occurrences
                (occurrence_key, automation_id, scheduled_for, state, skip_reason, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(occurrence_key) DO UPDATE SET
                state = excluded.state,
                skip_reason = excluded.skip_reason
            "#,
            params![
                occurrence.occurrence_key,
                occurrence.automation_id,
                occurrence.scheduled_for.unix_timestamp(),
                state,
                skip_reason,
                now.unix_timestamp(),
            ],
        )?;
        Ok(())
    }

    pub fn update_last_checked(
        &self,
        automation_id: &AutomationId,
        checked_at: OffsetDateTime,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE automations SET last_checked_at = ?2 WHERE id = ?1",
            params![automation_id, checked_at.unix_timestamp()],
        )?;
        Ok(())
    }

    pub fn upsert_run(&self, run: &AutomationRunSummary, updated_at: OffsetDateTime) -> Result<()> {
        let summary_json = serde_json::to_string(run)?;
        self.conn.execute(
            r#"
            INSERT INTO automation_runs
                (run_id, automation_id, occurrence_key, state, scheduled_for, summary_json, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(run_id) DO UPDATE SET
                state = excluded.state,
                summary_json = excluded.summary_json,
                updated_at = excluded.updated_at
            "#,
            params![
                run.run_id,
                run.automation_id,
                run.occurrence_key,
                run_state(run.state),
                run.scheduled_for.unix_timestamp(),
                summary_json,
                updated_at.unix_timestamp(),
            ],
        )?;
        Ok(())
    }

    pub fn get_run(&self, run_id: &AutomationRunId) -> Result<Option<AutomationRunSummary>> {
        self.conn
            .query_row(
                "SELECT summary_json FROM automation_runs WHERE run_id = ?1",
                params![run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|json| Ok(serde_json::from_str(&json)?))
            .transpose()
    }

    pub fn list_runs(
        &self,
        automation_id: &AutomationId,
        limit: Option<usize>,
    ) -> Result<Vec<AutomationRunSummary>> {
        let mut statement = self.conn.prepare(
            "SELECT summary_json FROM automation_runs WHERE automation_id = ?1 ORDER BY scheduled_for DESC, updated_at DESC",
        )?;
        let rows = statement.query_map(params![automation_id], |row| row.get::<_, String>(0))?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(serde_json::from_str(&row?)?);
            if limit.is_some_and(|limit| runs.len() >= limit) {
                break;
            }
        }
        Ok(runs)
    }

    pub fn append_log(&self, entry: &RunLogEntry) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO automation_run_logs (run_id, stream, chunk, timestamp)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![
                entry.run_id,
                entry.stream,
                entry.chunk,
                entry.timestamp.unix_timestamp(),
            ],
        )?;
        Ok(())
    }

    pub fn list_logs(&self, run_id: &AutomationRunId) -> Result<Vec<RunLogEntry>> {
        let mut statement = self.conn.prepare(
            "SELECT stream, chunk, timestamp FROM automation_run_logs WHERE run_id = ?1 ORDER BY id",
        )?;
        let rows = statement.query_map(params![run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;

        rows.map(|row| {
            let (stream, chunk, timestamp) = row?;
            Ok(RunLogEntry {
                run_id: run_id.clone(),
                stream,
                chunk,
                timestamp: offset_from_unix(timestamp)?,
            })
        })
        .collect()
    }
}

pub(crate) fn insert_lease(
    conn: &Connection,
    lease: &AutomationLeaseRecord,
) -> rusqlite::Result<usize> {
    conn.execute(
        r#"
        INSERT INTO automation_leases
            (run_id, automation_id, occurrence_key, server_id, server_role, leased_at, expires_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        params![
            lease.run_id,
            lease.automation_id,
            lease.occurrence_key,
            lease.server_id,
            lease.server_role,
            lease.leased_at.unix_timestamp(),
            lease.expires_at.unix_timestamp(),
        ],
    )
}

pub(crate) fn delete_expired_lease(
    conn: &Connection,
    occurrence_key: &str,
    now: OffsetDateTime,
) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM automation_leases WHERE occurrence_key = ?1 AND expires_at <= ?2",
        params![occurrence_key, now.unix_timestamp()],
    )
}

pub(crate) fn delete_lease(conn: &Connection, run_id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM automation_leases WHERE run_id = ?1",
        params![run_id],
    )
}

fn run_state(state: AutomationRunState) -> &'static str {
    match state {
        AutomationRunState::Scheduled => "scheduled",
        AutomationRunState::Leased => "leased",
        AutomationRunState::Queued => "queued",
        AutomationRunState::Running => "running",
        AutomationRunState::Completed => "completed",
        AutomationRunState::Failed => "failed",
        AutomationRunState::Skipped => "skipped",
        AutomationRunState::Cancelled => "cancelled",
    }
}

fn offset_from_unix(timestamp: i64) -> Result<OffsetDateTime> {
    Ok(OffsetDateTime::from_unix_timestamp(timestamp)?)
}
