//! Schema-versioned normalized JSONL export (and import for external
//! aggregation round-trips). Exported rows carry ids, names, timestamps,
//! durations, status, and counts only — never prompt/output text.

use std::io::Write;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::model::ANALYTICS_JSONL_SCHEMA_VERSION;
use crate::store::AnalyticsStore;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum AnalyticsJsonlRecord {
    #[serde(rename = "tool_call", rename_all = "camelCase")]
    ToolCall {
        schema_version: u32,
        thread_id: String,
        turn_id: String,
        tool_id: String,
        tool_name: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
        duration_ms: Option<i64>,
        status: String,
    },
    #[serde(rename = "token_usage", rename_all = "camelCase")]
    TokenUsage {
        schema_version: u32,
        thread_id: String,
        turn_id: String,
        provider: Option<String>,
        model: Option<String>,
        recorded_at: String,
        prompt_tokens: u32,
        completion_tokens: u32,
        total_tokens: u32,
        cached_prompt_tokens: u32,
    },
    #[serde(rename = "turn", rename_all = "camelCase")]
    Turn {
        schema_version: u32,
        thread_id: String,
        turn_id: String,
        provider: Option<String>,
        model: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
        status: String,
        error_kind: Option<String>,
    },
}

fn rfc3339(ms: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_default()
}

impl AnalyticsStore {
    /// Streams every turn, tool-call, and token-usage row as normalized
    /// JSONL into `output`. Returns the record count.
    pub fn export_jsonl(&self, output: &mut dyn Write) -> anyhow::Result<u64> {
        let conn = self.conn.lock().unwrap();
        let mut written = 0_u64;

        let mut statement = conn.prepare(
            "SELECT thread_id, turn_id, provider, model, started_at_ms, completed_at_ms, status, \
             error_kind FROM turns ORDER BY thread_id, turn_id",
        )?;
        let turns = statement.query_map([], |row| {
            Ok(AnalyticsJsonlRecord::Turn {
                schema_version: ANALYTICS_JSONL_SCHEMA_VERSION,
                thread_id: row.get(0)?,
                turn_id: row.get(1)?,
                provider: row.get(2)?,
                model: row.get(3)?,
                started_at: row.get::<_, Option<i64>>(4)?.map(rfc3339),
                completed_at: row.get::<_, Option<i64>>(5)?.map(rfc3339),
                status: row.get(6)?,
                error_kind: row.get(7)?,
            })
        })?;
        for record in turns {
            serde_json::to_writer(&mut *output, &record?)?;
            output.write_all(b"\n")?;
            written += 1;
        }

        let mut statement = conn.prepare(
            "SELECT thread_id, turn_id, tool_id, tool_name, started_at_ms, completed_at_ms, \
             duration_ms, status FROM tool_calls ORDER BY thread_id, turn_id, tool_id",
        )?;
        let tool_calls = statement.query_map([], |row| {
            Ok(AnalyticsJsonlRecord::ToolCall {
                schema_version: ANALYTICS_JSONL_SCHEMA_VERSION,
                thread_id: row.get(0)?,
                turn_id: row.get(1)?,
                tool_id: row.get(2)?,
                tool_name: row.get(3)?,
                started_at: row.get::<_, Option<i64>>(4)?.map(rfc3339),
                completed_at: row.get::<_, Option<i64>>(5)?.map(rfc3339),
                duration_ms: row.get(6)?,
                status: row.get(7)?,
            })
        })?;
        for record in tool_calls {
            serde_json::to_writer(&mut *output, &record?)?;
            output.write_all(b"\n")?;
            written += 1;
        }

        let mut statement = conn.prepare(
            "SELECT u.thread_id, u.turn_id, tu.provider, tu.model, u.recorded_at_ms, \
             u.prompt_tokens, u.completion_tokens, u.total_tokens, u.cached_prompt_tokens
             FROM token_usage u
             LEFT JOIN turns tu ON tu.thread_id = u.thread_id AND tu.turn_id = u.turn_id
             ORDER BY u.thread_id, u.turn_id",
        )?;
        let usage = statement.query_map([], |row| {
            Ok(AnalyticsJsonlRecord::TokenUsage {
                schema_version: ANALYTICS_JSONL_SCHEMA_VERSION,
                thread_id: row.get(0)?,
                turn_id: row.get(1)?,
                provider: row.get(2)?,
                model: row.get(3)?,
                recorded_at: rfc3339(row.get(4)?),
                prompt_tokens: row.get::<_, i64>(5)? as u32,
                completion_tokens: row.get::<_, i64>(6)? as u32,
                total_tokens: row.get::<_, i64>(7)? as u32,
                cached_prompt_tokens: row.get::<_, i64>(8)? as u32,
            })
        })?;
        for record in usage {
            serde_json::to_writer(&mut *output, &record?)?;
            output.write_all(b"\n")?;
            written += 1;
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TokenUsageRecord, ToolCallRecord, TurnRecord, WorkspaceLabelMode};

    #[test]
    fn jsonl_export_is_schema_versioned_and_body_free() {
        let dir =
            std::env::temp_dir().join(format!("roder-analytics-jsonl-{}", uuid::Uuid::new_v4()));
        let store = AnalyticsStore::open(
            &AnalyticsStore::default_path(&dir),
            WorkspaceLabelMode::FullPath,
        )
        .unwrap();
        store
            .upsert_turn(&TurnRecord {
                thread_id: "t1".into(),
                turn_id: "u1".into(),
                provider: Some("mock".into()),
                model: Some("mock".into()),
                runtime_profile: None,
                started_at_ms: Some(1_000),
                completed_at_ms: Some(2_000),
                status: "completed".into(),
                error_kind: None,
            })
            .unwrap();
        store
            .upsert_tool_call(&ToolCallRecord {
                thread_id: "t1".into(),
                turn_id: "u1".into(),
                tool_id: "call-1".into(),
                tool_name: Some("read_file".into()),
                started_at_ms: Some(1_100),
                completed_at_ms: Some(1_225),
                duration_ms: Some(125),
                status: "success".into(),
                is_error: false,
            })
            .unwrap();
        store
            .upsert_token_usage(&TokenUsageRecord {
                thread_id: "t1".into(),
                turn_id: "u1".into(),
                provider: None,
                model: None,
                recorded_at_ms: 2_000,
                prompt_tokens: 100,
                completion_tokens: 20,
                total_tokens: 120,
                cached_prompt_tokens: 80,
            })
            .unwrap();

        let mut output = Vec::new();
        let written = store.export_jsonl(&mut output).unwrap();
        assert_eq!(written, 3);
        let text = String::from_utf8(output).unwrap();
        let records: Vec<AnalyticsJsonlRecord> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        assert!(text.contains("\"schemaVersion\":1"));
        assert!(text.contains("\"kind\":\"tool_call\""));
        assert!(text.contains("\"durationMs\":125"));
        // No body-ish keys exist in the export vocabulary.
        for forbidden in ["output", "arguments", "prompt\"", "text\""] {
            assert!(!text.contains(forbidden), "export contains {forbidden}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
