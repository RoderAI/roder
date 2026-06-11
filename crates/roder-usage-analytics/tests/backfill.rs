//! Backfill tests (roadmap phase 73, Task 3): clean import, repeated
//! import with offsets, corrupt-line best-effort handling, and rebuild
//! equivalence — all against a fixture thread directory.

use std::path::PathBuf;

use roder_api::events::{
    EventEnvelope, EventSource, RoderEvent, ThreadCreated, ToolCallCompleted, ToolCallStarted,
    TurnCompleted, TurnStarted,
};
use roder_api::inference::TokenUsage;
use roder_usage_analytics::{
    AnalyticsStore, BackfillOptions, StatsFilter, WorkspaceLabelMode, backfill_analytics,
};
use time::OffsetDateTime;

fn at(ms: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).unwrap()
}

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

fn turn_events(thread_id: &str, turn_id: &str, base_ms: i64) -> Vec<EventEnvelope> {
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
                output: None,
                timestamp: at(base_ms + 225),
            }),
        ),
        envelope(
            5,
            RoderEvent::TurnCompleted(TurnCompleted {
                thread_id: thread_id.to_string(),
                turn_id: turn_id.to_string(),
                usage: Some(TokenUsage {
                    prompt_tokens: 100,
                    completion_tokens: 20,
                    total_tokens: 120,
                    cached_prompt_tokens: 80,
                    ..TokenUsage::default()
                }),
                finish_reason: Some("stop".to_string()),
                timestamp: at(base_ms + 500),
            }),
        ),
    ]
}

struct Fixture {
    thread_root: PathBuf,
    data_dir: PathBuf,
}

fn fixture(label: &str) -> Fixture {
    let base = std::env::temp_dir().join(format!(
        "roder-analytics-backfill-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    let thread_root = base.join("threads");
    let thread_dir = thread_root.join("thread-1");
    std::fs::create_dir_all(&thread_dir).unwrap();
    let events: Vec<String> = turn_events("thread-1", "turn-1", 1_750_000_000_000)
        .iter()
        .map(|event| serde_json::to_string(event).unwrap())
        .collect();
    std::fs::write(thread_dir.join("events.jsonl"), events.join("\n") + "\n").unwrap();
    std::fs::write(
        thread_dir.join("metadata.json"),
        serde_json::json!({
            "thread_id": "thread-1",
            "title": "Fixture",
            "workspace": "/home/user/projects/demo",
            "provider": "mock",
            "model": "mock-model",
            "tool_allowlist": [],
            "external_tools": [],
            "created_at": "2026-06-01T00:00:00Z",
            "updated_at": "2026-06-01T01:00:00Z",
            "message_count": 1
        })
        .to_string(),
    )
    .unwrap();
    Fixture {
        thread_root,
        data_dir: base.join("data"),
    }
}

fn open_store(fixture: &Fixture, mode: WorkspaceLabelMode) -> AnalyticsStore {
    AnalyticsStore::open(&AnalyticsStore::default_path(&fixture.data_dir), mode).unwrap()
}

#[tokio::test]
async fn analytics_backfill_is_idempotent_and_enriches_sessions() {
    let fixture = fixture("clean");
    let store = open_store(&fixture, WorkspaceLabelMode::BasenameOnly);

    let first = backfill_analytics(&fixture.thread_root, &store, BackfillOptions::default())
        .unwrap();
    assert_eq!(first.files_scanned, 1);
    assert_eq!(first.lines_ingested, 5);
    assert_eq!(first.sessions_enriched, 1);
    assert!(first.parse_errors.is_empty());
    let counts_first = store.counts().unwrap();
    assert_eq!(counts_first.turns, 1);
    assert_eq!(counts_first.tool_calls, 1);
    assert_eq!(counts_first.token_usage, 1);

    // Second run: offsets skip every line; aggregates unchanged.
    let second = backfill_analytics(&fixture.thread_root, &store, BackfillOptions::default())
        .unwrap();
    assert_eq!(second.lines_ingested, 0);
    assert_eq!(second.lines_skipped_by_offset, 5);
    assert_eq!(store.counts().unwrap(), counts_first);

    // Session enrichment applied the workspace label mode (basename only).
    let sessions = store.session_summaries(&StatsFilter::default()).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].workspace_label.as_deref(), Some("demo"));
    assert_eq!(sessions[0].provider.as_deref(), Some("mock"));
    assert_eq!(sessions[0].total_tokens, 120);
}

#[tokio::test]
async fn corrupt_lines_fail_closed_unless_best_effort_reports_them() {
    let fixture = fixture("corrupt");
    // Append one corrupt line and one valid event after it.
    let events_path = fixture.thread_root.join("thread-1/events.jsonl");
    let mut contents = std::fs::read_to_string(&events_path).unwrap();
    contents.push_str("{not json}\n");
    let extra = envelope(
        6,
        RoderEvent::ToolCallCompleted(ToolCallCompleted {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-2".to_string(),
            tool_id: "call-2".to_string(),
            tool_name: Some("shell".to_string()),
            display_payload: None,
            is_error: true,
            output: None,
            timestamp: at(1_750_000_100_000),
        }),
    );
    contents.push_str(&(serde_json::to_string(&extra).unwrap() + "\n"));
    std::fs::write(&events_path, contents).unwrap();

    // Strict mode fails with precise file/line evidence.
    let store = open_store(&fixture, WorkspaceLabelMode::FullPath);
    let error = backfill_analytics(&fixture.thread_root, &store, BackfillOptions::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("events.jsonl:6"), "{error}");

    // Best-effort skips the corrupt line, reports it, and ingests the rest.
    let report = backfill_analytics(
        &fixture.thread_root,
        &store,
        BackfillOptions {
            best_effort: true,
            rebuild: true,
        },
    )
    .unwrap();
    assert_eq!(report.parse_errors.len(), 1);
    assert_eq!(report.parse_errors[0].line, 6);
    assert!(report.parse_errors[0].path.ends_with("events.jsonl"));
    assert_eq!(report.lines_ingested, 6);
    assert_eq!(store.counts().unwrap().tool_calls, 2);
}

#[tokio::test]
async fn rebuild_produces_the_same_aggregates_as_a_first_import() {
    let fixture = fixture("rebuild");
    let store = open_store(&fixture, WorkspaceLabelMode::Hashed);

    backfill_analytics(&fixture.thread_root, &store, BackfillOptions::default()).unwrap();
    let first_counts = store.counts().unwrap();
    let first_summary = store.usage_summary(&StatsFilter::default()).unwrap();

    let rebuilt = backfill_analytics(
        &fixture.thread_root,
        &store,
        BackfillOptions {
            rebuild: true,
            best_effort: false,
        },
    )
    .unwrap();
    assert_eq!(rebuilt.lines_ingested, 5, "rebuild replays everything");
    assert_eq!(store.counts().unwrap(), first_counts);
    assert_eq!(store.usage_summary(&StatsFilter::default()).unwrap(), first_summary);

    // Hashed mode never exposes the raw path.
    let sessions = store.session_summaries(&StatsFilter::default()).unwrap();
    let label = sessions[0].workspace_label.clone().unwrap();
    assert!(!label.contains("projects"), "{label}");
    assert_eq!(label.len(), 16);
}
