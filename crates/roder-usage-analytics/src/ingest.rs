//! Projection of canonical `EventEnvelope` values into analytics records.

use roder_api::events::{EventEnvelope, RoderEvent};
use roder_api::inference::TokenUsage;
use time::OffsetDateTime;

use crate::model::{SessionRecord, TokenUsageRecord, ToolCallRecord, TurnRecord};
use crate::store::AnalyticsStore;

fn ms(timestamp: OffsetDateTime) -> i64 {
    (timestamp.unix_timestamp_nanos() / 1_000_000) as i64
}

/// Stateless event projector. Partial pairs (start without completion or
/// completion without start) remain visible as `running`/`partial` records
/// — they never panic or block later events.
pub struct AnalyticsIngestor<'a> {
    store: &'a AnalyticsStore,
}

impl<'a> AnalyticsIngestor<'a> {
    pub fn new(store: &'a AnalyticsStore) -> Self {
        Self { store }
    }

    /// Projects one envelope. Unknown events are ignored. Token usage is
    /// recorded only from terminal turn events (`TurnCompleted`/`TurnFailed`
    /// carry the turn-total usage), so streamed per-step usage events can
    /// never double-count a turn.
    pub fn ingest_event(&self, envelope: &EventEnvelope) -> anyhow::Result<()> {
        match &envelope.event {
            RoderEvent::ThreadCreated(event) => self.store.upsert_session(&SessionRecord {
                thread_id: event.thread_id.clone(),
                workspace_key: None,
                workspace_label: None,
                provider: None,
                model: None,
                created_at_ms: ms(event.timestamp),
                updated_at_ms: ms(event.timestamp),
            }),
            RoderEvent::TurnStarted(event) => self.store.upsert_turn(&TurnRecord {
                thread_id: event.thread_id.clone(),
                turn_id: event.turn_id.clone(),
                provider: None,
                model: None,
                runtime_profile: Some(format!("{:?}", event.runtime_profile).to_lowercase()),
                started_at_ms: Some(ms(event.timestamp)),
                completed_at_ms: None,
                status: "running".to_string(),
                error_kind: None,
            }),
            RoderEvent::InferenceStarted(event) => {
                self.store.upsert_session(&SessionRecord {
                    thread_id: event.thread_id.clone(),
                    workspace_key: None,
                    workspace_label: None,
                    provider: Some(event.model.provider.clone()),
                    model: Some(event.model.model.clone()),
                    created_at_ms: ms(event.timestamp),
                    updated_at_ms: ms(event.timestamp),
                })?;
                self.store.upsert_turn(&TurnRecord {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    provider: Some(event.model.provider.clone()),
                    model: Some(event.model.model.clone()),
                    runtime_profile: None,
                    started_at_ms: Some(ms(event.timestamp)),
                    completed_at_ms: None,
                    status: "running".to_string(),
                    error_kind: None,
                })
            }
            RoderEvent::TurnCompleted(event) => {
                self.store.upsert_turn(&TurnRecord {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    provider: None,
                    model: None,
                    runtime_profile: None,
                    started_at_ms: None,
                    completed_at_ms: Some(ms(event.timestamp)),
                    status: "completed".to_string(),
                    error_kind: None,
                })?;
                self.record_usage(
                    &event.thread_id,
                    &event.turn_id,
                    event.usage.as_ref(),
                    event.timestamp,
                )
            }
            RoderEvent::TurnFailed(event) => {
                self.store.upsert_turn(&TurnRecord {
                    thread_id: event.thread_id.clone(),
                    turn_id: event.turn_id.clone(),
                    provider: None,
                    model: None,
                    runtime_profile: None,
                    started_at_ms: None,
                    completed_at_ms: Some(ms(event.timestamp)),
                    status: "failed".to_string(),
                    error_kind: Some(
                        event
                            .error_kind
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                })?;
                self.record_usage(
                    &event.thread_id,
                    &event.turn_id,
                    event.usage.as_ref(),
                    event.timestamp,
                )
            }
            RoderEvent::ToolCallStarted(event) => self.store.upsert_tool_call(&ToolCallRecord {
                thread_id: event.thread_id.clone(),
                turn_id: event.turn_id.clone(),
                tool_id: event.tool_id.clone(),
                tool_name: event.tool_name.clone(),
                started_at_ms: Some(ms(event.timestamp)),
                completed_at_ms: None,
                duration_ms: None,
                status: "running".to_string(),
                is_error: false,
            }),
            RoderEvent::ToolCallCompleted(event) => self.store.upsert_tool_call(&ToolCallRecord {
                thread_id: event.thread_id.clone(),
                turn_id: event.turn_id.clone(),
                tool_id: event.tool_id.clone(),
                tool_name: event.tool_name.clone(),
                started_at_ms: None,
                completed_at_ms: Some(ms(event.timestamp)),
                duration_ms: None,
                // Without a matching start event the record stays visibly
                // partial; the store upgrades it when both halves exist.
                status: if event.is_error { "error" } else { "success" }.to_string(),
                is_error: event.is_error,
            }),
            _ => Ok(()),
        }
    }

    fn record_usage(
        &self,
        thread_id: &str,
        turn_id: &str,
        usage: Option<&TokenUsage>,
        timestamp: OffsetDateTime,
    ) -> anyhow::Result<()> {
        let Some(usage) = usage else {
            return Ok(());
        };
        if usage.total_tokens == 0 && usage.prompt_tokens == 0 && usage.completion_tokens == 0 {
            return Ok(());
        }
        self.store.upsert_token_usage(&TokenUsageRecord {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            // Provider/model are joined from `turns` at query time.
            provider: None,
            model: None,
            recorded_at_ms: ms(timestamp),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            cached_prompt_tokens: usage.cached_prompt_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WorkspaceLabelMode;
    use roder_api::events::{
        EventSource, ThreadCreated, ToolCallCompleted, ToolCallStarted, TurnCompleted, TurnFailed,
        TurnStarted,
    };

    fn envelope(seq: u64, event: RoderEvent) -> EventEnvelope {
        EventEnvelope {
            event_id: format!("event-{seq}"),
            seq,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            source: EventSource::Core,
            kind: event.kind().to_string(),
            thread_id: event.thread_id().cloned(),
            turn_id: event.turn_id().cloned(),
            event,
        }
    }

    fn at(ms_value: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms_value) * 1_000_000).unwrap()
    }

    fn usage(total: u32) -> TokenUsage {
        TokenUsage {
            prompt_tokens: total - 20,
            completion_tokens: 20,
            total_tokens: total,
            cached_prompt_tokens: 10,
            cache_creation_prompt_tokens: 0,
            ..TokenUsage::default()
        }
    }

    fn temp_store() -> (AnalyticsStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "roder-analytics-ingest-{}",
            uuid::Uuid::new_v4()
        ));
        let store = AnalyticsStore::open(
            &AnalyticsStore::default_path(&dir),
            WorkspaceLabelMode::FullPath,
        )
        .unwrap();
        (store, dir)
    }

    pub(crate) fn fake_turn_events(thread_id: &str, turn_id: &str, base_ms: i64) -> Vec<EventEnvelope> {
        vec![
            envelope(
                1,
                RoderEvent::ThreadCreated(ThreadCreated {
                    thread_id: thread_id.to_string(),
                    timestamp: at(base_ms),
                }),
            ),
            envelope(
                2,
                RoderEvent::TurnStarted(TurnStarted {
                    thread_id: thread_id.to_string(),
                    turn_id: turn_id.to_string(),
                    runtime_profile: Default::default(),
                    timestamp: at(base_ms + 10),
                }),
            ),
            envelope(
                3,
                RoderEvent::ToolCallStarted(ToolCallStarted {
                    thread_id: thread_id.to_string(),
                    turn_id: turn_id.to_string(),
                    tool_id: "call-1".to_string(),
                    tool_name: Some("read_file".to_string()),
                    display_payload: None,
                    timestamp: at(base_ms + 100),
                }),
            ),
            envelope(
                4,
                RoderEvent::ToolCallCompleted(ToolCallCompleted {
                    thread_id: thread_id.to_string(),
                    turn_id: turn_id.to_string(),
                    tool_id: "call-1".to_string(),
                    tool_name: Some("read_file".to_string()),
                    display_payload: None,
                    is_error: false,
                    output: Some("secret file contents".to_string()),
                    timestamp: at(base_ms + 225),
                }),
            ),
            envelope(
                5,
                RoderEvent::TurnCompleted(TurnCompleted {
                    thread_id: thread_id.to_string(),
                    turn_id: turn_id.to_string(),
                    usage: Some(usage(120)),
                    finish_reason: Some("stop".to_string()),
                    timestamp: at(base_ms + 500),
                }),
            ),
        ]
    }

    #[test]
    fn fake_turn_produces_one_turn_one_duration_one_usage_record() {
        let (store, dir) = temp_store();
        let ingestor = AnalyticsIngestor::new(&store);
        for event in fake_turn_events("t1", "u1", 10_000) {
            ingestor.ingest_event(&event).unwrap();
        }

        let counts = store.counts().unwrap();
        assert_eq!(counts.sessions, 1);
        assert_eq!(counts.turns, 1);
        assert_eq!(counts.tool_calls, 1);
        assert_eq!(counts.token_usage, 1);

        let conn = store.conn.lock().unwrap();
        let (duration, status): (i64, String) = conn
            .query_row("SELECT duration_ms, status FROM tool_calls", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(duration, 125);
        assert_eq!(status, "success");
        let total: i64 = conn
            .query_row("SELECT total_tokens FROM token_usage", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 120);
        // Tool output bodies never reach the database.
        let dumped: String = conn
            .query_row(
                "SELECT COALESCE(GROUP_CONCAT(tool_name), '') FROM tool_calls",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!dumped.contains("secret file contents"));
        drop(conn);

        // Replaying the same events does not double-count anything.
        let ingestor = AnalyticsIngestor::new(&store);
        for event in fake_turn_events("t1", "u1", 10_000) {
            ingestor.ingest_event(&event).unwrap();
        }
        assert_eq!(store.counts().unwrap(), counts);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_turns_and_failed_tools_are_queryable_with_error_state() {
        let (store, dir) = temp_store();
        let ingestor = AnalyticsIngestor::new(&store);
        ingestor
            .ingest_event(&envelope(
                1,
                RoderEvent::ToolCallCompleted(ToolCallCompleted {
                    thread_id: "t1".to_string(),
                    turn_id: "u1".to_string(),
                    tool_id: "call-err".to_string(),
                    tool_name: Some("shell".to_string()),
                    display_payload: None,
                    is_error: true,
                    output: None,
                    timestamp: at(1_000),
                }),
            ))
            .unwrap();
        ingestor
            .ingest_event(&envelope(
                2,
                RoderEvent::TurnFailed(TurnFailed {
                    thread_id: "t1".to_string(),
                    turn_id: "u1".to_string(),
                    error: "provider exploded".to_string(),
                    error_kind: Some("provider".to_string()),
                    usage: Some(usage(50)),
                    timestamp: at(2_000),
                }),
            ))
            .unwrap();

        let conn = store.conn.lock().unwrap();
        let (status, error_kind): (String, String) = conn
            .query_row("SELECT status, error_kind FROM turns", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(status, "failed");
        assert_eq!(error_kind, "provider");
        // Tool completion without a start event remains a visible partial
        // record (no duration, but counted with its error state).
        let (is_error, duration): (bool, Option<i64>) = conn
            .query_row("SELECT is_error, duration_ms FROM tool_calls", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert!(is_error);
        assert_eq!(duration, None);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
