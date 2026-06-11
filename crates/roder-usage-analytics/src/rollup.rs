//! Daily rollup generation: cached per-day/tool/provider/model/workspace
//! aggregates for fast dashboards. Raw `tool_calls`/`token_usage` rows stay
//! the source of truth; rollups are recomputed from them on refresh.

use rusqlite::params;

use crate::model::DailyRollupRow;
use crate::query::percentile;
use crate::store::AnalyticsStore;

impl AnalyticsStore {
    /// Recomputes all daily rollups from raw rows. Deterministic and
    /// idempotent: the rollup table always reflects the raw tables.
    pub fn refresh_daily_rollups(&self) -> anyhow::Result<u64> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM daily_rollups", [])?;

        // Tool-call aggregates per (day, workspace, provider, model, tool).
        let mut grouped: std::collections::BTreeMap<
            (String, String, String, String, String),
            (Vec<i64>, u64, u64),
        > = std::collections::BTreeMap::new();
        {
            let mut statement = conn.prepare(
                "SELECT strftime('%Y-%m-%d', COALESCE(tc.started_at_ms, tc.completed_at_ms) / 1000, \
                 'unixepoch'),
                        COALESCE(s.workspace_key, ''), COALESCE(tu.provider, ''),
                        COALESCE(tu.model, ''), COALESCE(tc.tool_name, ''),
                        tc.duration_ms, tc.is_error
                 FROM tool_calls tc
                 LEFT JOIN turns tu ON tu.thread_id = tc.thread_id AND tu.turn_id = tc.turn_id
                 LEFT JOIN sessions s ON s.thread_id = tc.thread_id
                 WHERE COALESCE(tc.started_at_ms, tc.completed_at_ms) IS NOT NULL",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    (
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ),
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, bool>(6)?,
                ))
            })?;
            for row in rows {
                let (key, duration, is_error) = row?;
                let entry = grouped.entry(key).or_default();
                entry.1 += 1;
                if is_error {
                    entry.2 += 1;
                }
                if let Some(duration) = duration {
                    entry.0.push(duration);
                }
            }
        }
        let mut written = 0_u64;
        for ((day, workspace, provider, model, tool), (mut durations, calls, errors)) in grouped {
            durations.sort_unstable();
            conn.execute(
                "INSERT INTO daily_rollups (day, workspace_key, provider, model, tool_name, \
                 call_count, error_count, total_duration_ms, p50_duration_ms, p95_duration_ms, \
                 p99_duration_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    day,
                    workspace,
                    provider,
                    model,
                    tool,
                    calls as i64,
                    errors as i64,
                    durations.iter().sum::<i64>(),
                    percentile(&durations, 0.50),
                    percentile(&durations, 0.95),
                    percentile(&durations, 0.99),
                ],
            )?;
            written += 1;
        }

        // Token aggregates land on the tool_name = '' row per group.
        let mut statement = conn.prepare(
            "SELECT strftime('%Y-%m-%d', u.recorded_at_ms / 1000, 'unixepoch'),
                    COALESCE(s.workspace_key, ''), COALESCE(tu.provider, ''),
                    COALESCE(tu.model, ''),
                    SUM(u.prompt_tokens), SUM(u.completion_tokens), SUM(u.total_tokens),
                    SUM(u.cached_prompt_tokens)
             FROM token_usage u
             LEFT JOIN turns tu ON tu.thread_id = u.thread_id AND tu.turn_id = u.turn_id
             LEFT JOIN sessions s ON s.thread_id = u.thread_id
             GROUP BY 1, 2, 3, 4",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?;
        for row in rows {
            let (day, workspace, provider, model, prompt, completion, total, cached) = row?;
            conn.execute(
                "INSERT INTO daily_rollups (day, workspace_key, provider, model, tool_name, \
                 prompt_tokens, completion_tokens, total_tokens, cached_prompt_tokens)
                 VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, ?7, ?8)
                 ON CONFLICT(day, workspace_key, provider, model, tool_name) DO UPDATE SET
                   prompt_tokens = excluded.prompt_tokens,
                   completion_tokens = excluded.completion_tokens,
                   total_tokens = excluded.total_tokens,
                   cached_prompt_tokens = excluded.cached_prompt_tokens",
                params![day, workspace, provider, model, prompt, completion, total, cached],
            )?;
            written += 1;
        }
        Ok(written)
    }

    pub fn daily_rollups(&self) -> anyhow::Result<Vec<DailyRollupRow>> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT day, workspace_key, provider, model, tool_name, call_count, error_count, \
             total_duration_ms, p50_duration_ms, p95_duration_ms, p99_duration_ms, prompt_tokens, \
             completion_tokens, total_tokens, cached_prompt_tokens
             FROM daily_rollups ORDER BY day, tool_name",
        )?;
        let rows = statement.query_map([], |row| {
            let optional = |value: String| if value.is_empty() { None } else { Some(value) };
            Ok(DailyRollupRow {
                day: row.get(0)?,
                workspace_key: optional(row.get(1)?),
                provider: optional(row.get(2)?),
                model: optional(row.get(3)?),
                tool_name: optional(row.get(4)?),
                call_count: row.get::<_, i64>(5)? as u64,
                error_count: row.get::<_, i64>(6)? as u64,
                total_duration_ms: row.get(7)?,
                p50_duration_ms: row.get(8)?,
                p95_duration_ms: row.get(9)?,
                p99_duration_ms: row.get(10)?,
                prompt_tokens: row.get::<_, i64>(11)? as u64,
                completion_tokens: row.get::<_, i64>(12)? as u64,
                total_tokens: row.get::<_, i64>(13)? as u64,
                cached_prompt_tokens: row.get::<_, i64>(14)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ToolCallRecord, WorkspaceLabelMode};

    #[test]
    fn rollup_refresh_is_deterministic_and_idempotent() {
        let dir = std::env::temp_dir().join(format!(
            "roder-analytics-rollup-{}",
            uuid::Uuid::new_v4()
        ));
        let store = AnalyticsStore::open(
            &AnalyticsStore::default_path(&dir),
            WorkspaceLabelMode::FullPath,
        )
        .unwrap();
        let day = 1_750_000_000_000_i64;
        for (index, duration) in (1..=20).enumerate() {
            store
                .upsert_tool_call(&ToolCallRecord {
                    thread_id: "t1".into(),
                    turn_id: "u1".into(),
                    tool_id: format!("call-{index}"),
                    tool_name: Some("grep".into()),
                    started_at_ms: Some(day),
                    completed_at_ms: Some(day + duration),
                    duration_ms: Some(duration),
                    status: "success".into(),
                    is_error: false,
                })
                .unwrap();
        }

        let first = store.refresh_daily_rollups().unwrap();
        let rows_first = store.daily_rollups().unwrap();
        let second = store.refresh_daily_rollups().unwrap();
        let rows_second = store.daily_rollups().unwrap();
        assert_eq!(first, second);
        assert_eq!(rows_first, rows_second);

        let grep = rows_first
            .iter()
            .find(|row| row.tool_name.as_deref() == Some("grep"))
            .unwrap();
        assert_eq!(grep.call_count, 20);
        assert_eq!(grep.p50_duration_ms, Some(10));
        assert_eq!(grep.p95_duration_ms, Some(19));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
