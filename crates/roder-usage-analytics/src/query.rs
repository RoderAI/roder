//! Typed query APIs: tool summaries with exact percentiles, token
//! summaries by grouping, session summaries, and underutilization helpers.

use rusqlite::{Connection, ToSql, params_from_iter};

use crate::model::{
    SessionSummary, StatsFilter, TokenGroup, TokenSummaryRow, ToolSummary, UsageSummary,
};
use crate::store::AnalyticsStore;

pub const DEFAULT_LIMIT: u64 = 50;
/// Hard cap applied to all listing queries regardless of the caller.
pub const MAX_LIMIT: u64 = 1_000;

fn effective_limit(filter: &StatsFilter) -> u64 {
    filter.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT)
}

struct Where {
    clause: String,
    params: Vec<Box<dyn ToSql>>,
}

/// Builds a WHERE clause over `tool_calls` (alias `tc`) joined with
/// `turns` (alias `tu`) and `sessions` (alias `s`).
fn tool_call_filter(filter: &StatsFilter) -> Where {
    let mut clauses = vec!["1=1".to_string()];
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(since) = filter.since_ms {
        clauses.push("COALESCE(tc.started_at_ms, tc.completed_at_ms) >= ?".to_string());
        params.push(Box::new(since));
    }
    if let Some(until) = filter.until_ms {
        clauses.push("COALESCE(tc.started_at_ms, tc.completed_at_ms) < ?".to_string());
        params.push(Box::new(until));
    }
    if let Some(thread) = &filter.thread_id {
        clauses.push("tc.thread_id = ?".to_string());
        params.push(Box::new(thread.clone()));
    }
    if let Some(tool) = &filter.tool_name {
        clauses.push("tc.tool_name = ?".to_string());
        params.push(Box::new(tool.clone()));
    }
    if let Some(provider) = &filter.provider {
        clauses.push("tu.provider = ?".to_string());
        params.push(Box::new(provider.clone()));
    }
    if let Some(model) = &filter.model {
        clauses.push("tu.model = ?".to_string());
        params.push(Box::new(model.clone()));
    }
    if let Some(workspace) = &filter.workspace_key {
        clauses.push("s.workspace_key = ?".to_string());
        params.push(Box::new(workspace.clone()));
    }
    Where {
        clause: clauses.join(" AND "),
        params,
    }
}

/// Exact nearest-rank percentile over a sorted ascending slice.
pub(crate) fn percentile(sorted: &[i64], quantile: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (quantile * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

impl AnalyticsStore {
    /// Per-tool summaries with exact percentiles computed from raw
    /// durations in the filtered window.
    pub fn tool_summaries(&self, filter: &StatsFilter) -> anyhow::Result<Vec<ToolSummary>> {
        let conn = self.conn.lock().unwrap();
        let where_clause = tool_call_filter(filter);
        let sql = format!(
            "SELECT COALESCE(tc.tool_name, '(unknown)') AS name, tc.duration_ms, tc.is_error
             FROM tool_calls tc
             LEFT JOIN turns tu ON tu.thread_id = tc.thread_id AND tu.turn_id = tc.turn_id
             LEFT JOIN sessions s ON s.thread_id = tc.thread_id
             WHERE {}",
            where_clause.clause
        );
        let mut statement = conn.prepare(&sql)?;
        let mut grouped: std::collections::BTreeMap<String, (Vec<i64>, u64, u64)> =
            std::collections::BTreeMap::new();
        let rows = statement.query_map(params_from_iter(where_clause.params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        for row in rows {
            let (name, duration, is_error) = row?;
            let entry = grouped.entry(name).or_default();
            entry.1 += 1;
            if is_error {
                entry.2 += 1;
            }
            if let Some(duration) = duration {
                entry.0.push(duration);
            }
        }
        let min_calls = filter.min_calls.unwrap_or(0);
        let mut summaries: Vec<ToolSummary> = grouped
            .into_iter()
            .filter(|(_, (_, calls, _))| *calls >= min_calls)
            .map(|(tool_name, (mut durations, call_count, error_count))| {
                durations.sort_unstable();
                let total: i64 = durations.iter().sum();
                ToolSummary {
                    tool_name,
                    call_count,
                    error_count,
                    error_rate: if call_count == 0 {
                        0.0
                    } else {
                        error_count as f64 / call_count as f64
                    },
                    total_duration_ms: total,
                    avg_duration_ms: if durations.is_empty() {
                        None
                    } else {
                        Some(total as f64 / durations.len() as f64)
                    },
                    p50_duration_ms: percentile(&durations, 0.50),
                    p95_duration_ms: percentile(&durations, 0.95),
                    p99_duration_ms: percentile(&durations, 0.99),
                }
            })
            .collect();
        summaries.sort_by(|a, b| b.call_count.cmp(&a.call_count));
        summaries.truncate(effective_limit(filter) as usize);
        Ok(summaries)
    }

    /// Token summaries grouped by day, session, provider, model, or
    /// workspace. Reads only the projected tables, never raw JSONL.
    pub fn token_summaries(
        &self,
        group: TokenGroup,
        filter: &StatsFilter,
    ) -> anyhow::Result<Vec<TokenSummaryRow>> {
        let group_expr = match group {
            TokenGroup::Day => {
                "strftime('%Y-%m-%d', token_usage.recorded_at_ms / 1000, 'unixepoch')"
            }
            TokenGroup::Session => "token_usage.thread_id",
            TokenGroup::Provider => "COALESCE(tu.provider, '(unknown)')",
            TokenGroup::Model => "COALESCE(tu.model, '(unknown)')",
            TokenGroup::Workspace => "COALESCE(s.workspace_label, '(unknown)')",
        };
        let mut clauses = vec!["1=1".to_string()];
        let mut params: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(since) = filter.since_ms {
            clauses.push("token_usage.recorded_at_ms >= ?".to_string());
            params.push(Box::new(since));
        }
        if let Some(until) = filter.until_ms {
            clauses.push("token_usage.recorded_at_ms < ?".to_string());
            params.push(Box::new(until));
        }
        if let Some(thread) = &filter.thread_id {
            clauses.push("token_usage.thread_id = ?".to_string());
            params.push(Box::new(thread.clone()));
        }
        if let Some(provider) = &filter.provider {
            clauses.push("tu.provider = ?".to_string());
            params.push(Box::new(provider.clone()));
        }
        if let Some(model) = &filter.model {
            clauses.push("tu.model = ?".to_string());
            params.push(Box::new(model.clone()));
        }
        if let Some(workspace) = &filter.workspace_key {
            clauses.push("s.workspace_key = ?".to_string());
            params.push(Box::new(workspace.clone()));
        }
        let sql = format!(
            "SELECT {group_expr} AS grp,
                    SUM(token_usage.prompt_tokens),
                    SUM(token_usage.completion_tokens),
                    SUM(token_usage.total_tokens),
                    SUM(token_usage.cached_prompt_tokens),
                    COUNT(*)
             FROM token_usage
             LEFT JOIN turns tu ON tu.thread_id = token_usage.thread_id
                               AND tu.turn_id = token_usage.turn_id
             LEFT JOIN sessions s ON s.thread_id = token_usage.thread_id
             WHERE {}
             GROUP BY grp
             ORDER BY SUM(token_usage.total_tokens) DESC
             LIMIT {}",
            clauses.join(" AND "),
            effective_limit(filter)
        );
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(params.iter()), |row| {
            Ok(TokenSummaryRow {
                group: row.get(0)?,
                prompt_tokens: row.get::<_, i64>(1)? as u64,
                completion_tokens: row.get::<_, i64>(2)? as u64,
                total_tokens: row.get::<_, i64>(3)? as u64,
                cached_prompt_tokens: row.get::<_, i64>(4)? as u64,
                turn_count: row.get::<_, i64>(5)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Session summaries sortable by tokens, tool calls, duration, errors.
    pub fn session_summaries(&self, filter: &StatsFilter) -> anyhow::Result<Vec<SessionSummary>> {
        let mut clauses = vec!["1=1".to_string()];
        let mut params: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(thread) = &filter.thread_id {
            clauses.push("s.thread_id = ?".to_string());
            params.push(Box::new(thread.clone()));
        }
        if let Some(workspace) = &filter.workspace_key {
            clauses.push("s.workspace_key = ?".to_string());
            params.push(Box::new(workspace.clone()));
        }
        if let Some(since) = filter.since_ms {
            clauses.push("s.updated_at_ms >= ?".to_string());
            params.push(Box::new(since));
        }
        if let Some(until) = filter.until_ms {
            clauses.push("s.created_at_ms < ?".to_string());
            params.push(Box::new(until));
        }
        let sql = format!(
            "SELECT s.thread_id, s.workspace_label, s.provider, s.model,
                    (SELECT COUNT(*) FROM turns t WHERE t.thread_id = s.thread_id),
                    (SELECT COUNT(*) FROM tool_calls tc WHERE tc.thread_id = s.thread_id),
                    (SELECT COUNT(*) FROM tool_calls tc
                      WHERE tc.thread_id = s.thread_id AND tc.is_error = 1),
                    (SELECT COALESCE(SUM(u.total_tokens), 0) FROM token_usage u
                      WHERE u.thread_id = s.thread_id),
                    (SELECT COALESCE(SUM(tc.duration_ms), 0) FROM tool_calls tc
                      WHERE tc.thread_id = s.thread_id),
                    (SELECT MIN(t.started_at_ms) FROM turns t WHERE t.thread_id = s.thread_id),
                    (SELECT MAX(t.completed_at_ms) FROM turns t WHERE t.thread_id = s.thread_id)
             FROM sessions s
             WHERE {}
             ORDER BY 8 DESC
             LIMIT {}",
            clauses.join(" AND "),
            effective_limit(filter)
        );
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(params.iter()), |row| {
            Ok(SessionSummary {
                thread_id: row.get(0)?,
                workspace_label: row.get(1)?,
                provider: row.get(2)?,
                model: row.get(3)?,
                turn_count: row.get::<_, i64>(4)? as u64,
                tool_call_count: row.get::<_, i64>(5)? as u64,
                tool_error_count: row.get::<_, i64>(6)? as u64,
                total_tokens: row.get::<_, i64>(7)? as u64,
                total_tool_duration_ms: row.get(8)?,
                first_activity_ms: row.get(9)?,
                last_activity_ms: row.get(10)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Overall window summary for dashboards and `stats summary`.
    pub fn usage_summary(&self, filter: &StatsFilter) -> anyhow::Result<UsageSummary> {
        let tools = self.tool_summaries(&StatsFilter {
            limit: Some(MAX_LIMIT),
            min_calls: None,
            ..filter.clone()
        })?;
        let tool_call_count = tools.iter().map(|tool| tool.call_count).sum();
        let tool_error_count = tools.iter().map(|tool| tool.error_count).sum();
        let most_called_tool = tools.first().map(|tool| tool.tool_name.clone());

        let mut clauses = vec!["1=1".to_string()];
        let mut params: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(since) = filter.since_ms {
            clauses.push("COALESCE(started_at_ms, completed_at_ms) >= ?".to_string());
            params.push(Box::new(since));
        }
        if let Some(until) = filter.until_ms {
            clauses.push("COALESCE(started_at_ms, completed_at_ms) < ?".to_string());
            params.push(Box::new(until));
        }
        if let Some(thread) = &filter.thread_id {
            clauses.push("thread_id = ?".to_string());
            params.push(Box::new(thread.clone()));
        }
        let conn = self.conn.lock().unwrap();
        let (turn_count, completed, failed): (i64, i64, i64) = conn.query_row(
            &format!(
                "SELECT COUNT(*),
                        SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END)
                 FROM turns WHERE {}",
                clauses.join(" AND ")
            ),
            params_from_iter(params.iter()),
            |row| {
                Ok((
                    row.get(0)?,
                    row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                ))
            },
        )?;

        let mut usage_clauses = vec!["1=1".to_string()];
        let mut usage_params: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(since) = filter.since_ms {
            usage_clauses.push("recorded_at_ms >= ?".to_string());
            usage_params.push(Box::new(since));
        }
        if let Some(until) = filter.until_ms {
            usage_clauses.push("recorded_at_ms < ?".to_string());
            usage_params.push(Box::new(until));
        }
        if let Some(thread) = &filter.thread_id {
            usage_clauses.push("thread_id = ?".to_string());
            usage_params.push(Box::new(thread.clone()));
        }
        let (prompt, completion, total, cached): (i64, i64, i64, i64) = conn.query_row(
            &format!(
                "SELECT COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0),
                        COALESCE(SUM(total_tokens), 0), COALESCE(SUM(cached_prompt_tokens), 0)
                 FROM token_usage WHERE {}",
                usage_clauses.join(" AND ")
            ),
            params_from_iter(usage_params.iter()),
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let session_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;

        Ok(UsageSummary {
            turn_count: turn_count as u64,
            completed_turn_count: completed as u64,
            failed_turn_count: failed as u64,
            tool_call_count,
            tool_error_count,
            prompt_tokens: prompt as u64,
            completion_tokens: completion as u64,
            total_tokens: total as u64,
            cached_prompt_tokens: cached as u64,
            session_count: session_count as u64,
            most_called_tool,
        })
    }

    /// Registered tools that were never called in the window.
    pub fn never_used_tools(
        &self,
        registered: &[String],
        filter: &StatsFilter,
    ) -> anyhow::Result<Vec<String>> {
        let used: std::collections::BTreeSet<String> = self
            .tool_summaries(&StatsFilter {
                limit: Some(MAX_LIMIT),
                min_calls: None,
                ..filter.clone()
            })?
            .into_iter()
            .map(|summary| summary.tool_name)
            .collect();
        Ok(registered
            .iter()
            .filter(|tool| !used.contains(*tool))
            .cloned()
            .collect())
    }
}

/// Sort orders shared by the CLI and app-server tool listings.
pub fn sort_tool_summaries(summaries: &mut [ToolSummary], sort: &str) {
    match sort {
        "p95" => summaries.sort_by(|a, b| b.p95_duration_ms.cmp(&a.p95_duration_ms)),
        "errors" => summaries.sort_by(|a, b| {
            b.error_rate
                .partial_cmp(&a.error_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "underused" => summaries.sort_by(|a, b| a.call_count.cmp(&b.call_count)),
        _ => summaries.sort_by(|a, b| b.call_count.cmp(&a.call_count)),
    }
}

pub(crate) fn _connection_for_tests(store: &AnalyticsStore) -> std::sync::MutexGuard<'_, Connection> {
    store.conn.lock().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TokenUsageRecord, ToolCallRecord, TurnRecord, WorkspaceLabelMode};

    fn temp_store() -> (AnalyticsStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "roder-analytics-query-{}",
            uuid::Uuid::new_v4()
        ));
        let store = AnalyticsStore::open(
            &AnalyticsStore::default_path(&dir),
            WorkspaceLabelMode::FullPath,
        )
        .unwrap();
        (store, dir)
    }

    fn seed_tool_calls(store: &AnalyticsStore, tool: &str, durations: &[i64], errors: u64) {
        for (index, duration) in durations.iter().enumerate() {
            store
                .upsert_tool_call(&ToolCallRecord {
                    thread_id: "t1".into(),
                    turn_id: "u1".into(),
                    tool_id: format!("{tool}-{index}"),
                    tool_name: Some(tool.to_string()),
                    started_at_ms: Some(1_000),
                    completed_at_ms: Some(1_000 + duration),
                    duration_ms: Some(*duration),
                    status: if (index as u64) < errors {
                        "error".into()
                    } else {
                        "success".into()
                    },
                    is_error: (index as u64) < errors,
                })
                .unwrap();
        }
    }

    #[test]
    fn percentiles_use_exact_nearest_rank() {
        // 1..=100 -> p50 = 50, p95 = 95, p99 = 99.
        let durations: Vec<i64> = (1..=100).collect();
        assert_eq!(percentile(&durations, 0.50), Some(50));
        assert_eq!(percentile(&durations, 0.95), Some(95));
        assert_eq!(percentile(&durations, 0.99), Some(99));
        assert_eq!(percentile(&[], 0.95), None);
        assert_eq!(percentile(&[7], 0.95), Some(7));
    }

    #[test]
    fn tool_summaries_compute_counts_errors_and_p95() {
        let (store, dir) = temp_store();
        seed_tool_calls(&store, "read_file", &(1..=100).collect::<Vec<_>>(), 5);
        seed_tool_calls(&store, "shell", &[10, 20], 2);

        let summaries = store.tool_summaries(&StatsFilter::default()).unwrap();
        assert_eq!(summaries[0].tool_name, "read_file");
        assert_eq!(summaries[0].call_count, 100);
        assert_eq!(summaries[0].error_count, 5);
        assert_eq!(summaries[0].p50_duration_ms, Some(50));
        assert_eq!(summaries[0].p95_duration_ms, Some(95));
        assert_eq!(summaries[0].p99_duration_ms, Some(99));
        assert_eq!(summaries[1].tool_name, "shell");
        assert!((summaries[1].error_rate - 1.0).abs() < f64::EPSILON);

        // Sort helpers are deterministic.
        let mut by_errors = summaries.clone();
        sort_tool_summaries(&mut by_errors, "errors");
        assert_eq!(by_errors[0].tool_name, "shell");
        let mut underused = summaries.clone();
        sort_tool_summaries(&mut underused, "underused");
        assert_eq!(underused[0].tool_name, "shell");

        // Never-used helper compares against the registered tool list.
        let registered = vec![
            "read_file".to_string(),
            "shell".to_string(),
            "write_file".to_string(),
        ];
        assert_eq!(
            store
                .never_used_tools(&registered, &StatsFilter::default())
                .unwrap(),
            vec!["write_file".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn token_summaries_group_by_day_and_session() {
        let (store, dir) = temp_store();
        let day1 = 1_750_000_000_000_i64; // 2025-06-15
        let day2 = day1 + 86_400_000;
        for (thread, turn, at, total) in [
            ("t1", "u1", day1, 100_u32),
            ("t1", "u2", day1 + 1_000, 50),
            ("t2", "u1", day2, 30),
        ] {
            store
                .upsert_turn(&TurnRecord {
                    thread_id: thread.into(),
                    turn_id: turn.into(),
                    provider: Some("mock".into()),
                    model: Some("mock-model".into()),
                    runtime_profile: None,
                    started_at_ms: Some(at),
                    completed_at_ms: Some(at + 10),
                    status: "completed".into(),
                    error_kind: None,
                })
                .unwrap();
            store
                .upsert_token_usage(&TokenUsageRecord {
                    thread_id: thread.into(),
                    turn_id: turn.into(),
                    provider: None,
                    model: None,
                    recorded_at_ms: at,
                    prompt_tokens: total - 10,
                    completion_tokens: 10,
                    total_tokens: total,
                    cached_prompt_tokens: 0,
                })
                .unwrap();
        }

        let by_day = store
            .token_summaries(TokenGroup::Day, &StatsFilter::default())
            .unwrap();
        assert_eq!(by_day.len(), 2);
        assert_eq!(by_day[0].total_tokens, 150);
        assert_eq!(by_day[0].turn_count, 2);

        let by_session = store
            .token_summaries(TokenGroup::Session, &StatsFilter::default())
            .unwrap();
        assert_eq!(by_session[0].group, "t1");
        assert_eq!(by_session[0].total_tokens, 150);

        let by_model = store
            .token_summaries(TokenGroup::Model, &StatsFilter::default())
            .unwrap();
        assert_eq!(by_model[0].group, "mock-model");
        assert_eq!(by_model[0].total_tokens, 180);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
