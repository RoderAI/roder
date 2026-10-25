use super::*;
use crate::model::{StatsFilter, TokenGroup, WorkspaceLabelMode};
use roder_api::events::{EventSource, TurnInterrupted, TurnStarted};
use serde_json::json;

fn event(usage: Option<TokenUsage>) -> EventEnvelope {
    let interruption: TurnInterrupted = serde_json::from_value(json!({
        "thread_id": "thread",
        "turn_id": "interrupted",
        "usage": usage,
        "timestamp": "1970-01-01T00:00:01Z"
    }))
    .unwrap();
    let event = RoderEvent::TurnInterrupted(interruption);
    EventEnvelope {
        event_id: "interruption".into(),
        seq: 1,
        timestamp: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1),
        source: EventSource::Core,
        kind: event.kind().into(),
        thread_id: event.thread_id().cloned(),
        turn_id: event.turn_id().cloned(),
        event,
    }
}

#[test]
fn interrupted_usage_is_persisted_queryable_and_replay_safe() {
    let dir = std::env::temp_dir().join(format!("roder-interrupted-{}", uuid::Uuid::new_v4()));
    let path = AnalyticsStore::default_path(&dir);
    let store = AnalyticsStore::open(&path, WorkspaceLabelMode::FullPath).unwrap();
    let usage = TokenUsage {
        prompt_tokens: 100,
        completion_tokens: 20,
        total_tokens: 120,
        cached_prompt_tokens: 30,
        ..TokenUsage::default()
    };
    let event = event(Some(usage));
    let ingestor = AnalyticsIngestor::new(&store);
    ingestor.ingest_event(&event).unwrap();
    ingestor.ingest_event(&event).unwrap();
    let mut delayed_start = event.clone();
    delayed_start.event = RoderEvent::TurnStarted(TurnStarted {
        thread_id: "thread".into(),
        turn_id: "interrupted".into(),
        runtime_profile: Default::default(),
        timestamp: OffsetDateTime::UNIX_EPOCH,
    });
    ingestor.ingest_event(&delayed_start).unwrap();
    assert_eq!(store.counts().unwrap().turns, 1);
    assert_eq!(store.counts().unwrap().token_usage, 1);
    drop(store);

    let store = AnalyticsStore::open(&path, WorkspaceLabelMode::FullPath).unwrap();
    let filter = StatsFilter {
        thread_id: Some("thread".into()),
        ..Default::default()
    };
    let rows = store.token_summaries(TokenGroup::Session, &filter).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].group, "thread");
    assert_eq!(rows[0].prompt_tokens, 100);
    assert_eq!(rows[0].completion_tokens, 20);
    assert_eq!(rows[0].total_tokens, 120);
    assert_eq!(rows[0].cached_prompt_tokens, 30);
    assert_eq!(rows[0].turn_count, 1);
    let summary = store.usage_summary(&filter).unwrap();
    assert_eq!(summary.turn_count, 1);
    assert_eq!(summary.completed_turn_count, 0);
    assert_eq!(summary.failed_turn_count, 0);
    let conn = store.conn.lock().unwrap();
    let (status, completed_at, error): (String, i64, Option<String>) = conn
        .query_row(
            "SELECT status, completed_at_ms, error_kind FROM turns",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "interrupted");
    assert_eq!(completed_at, 1000);
    assert_eq!(error, None);
    drop(conn);
    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn interruption_without_usage_does_not_invent_a_token_record() {
    let dir = std::env::temp_dir().join(format!("roder-interrupted-{}", uuid::Uuid::new_v4()));
    let store = AnalyticsStore::open(
        &AnalyticsStore::default_path(&dir),
        WorkspaceLabelMode::FullPath,
    )
    .unwrap();
    AnalyticsIngestor::new(&store)
        .ingest_event(&event(None))
        .unwrap();
    assert_eq!(store.counts().unwrap().turns, 1);
    assert_eq!(store.counts().unwrap().token_usage, 0);
    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}
