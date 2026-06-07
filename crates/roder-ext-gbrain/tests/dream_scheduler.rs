use std::sync::Arc;

use roder_api::memory::MemoryScope;
use roder_ext_gbrain::{
    CaptureInput, DreamMode, DreamScheduleConfig, Embedder, GbrainStore, ScheduledDreamOutcome,
    ScheduledDreamSkipReason, run_scheduled_dream_once,
};
use time::Duration;

fn store() -> Arc<GbrainStore> {
    Arc::new(GbrainStore::open_in_memory(Embedder::new(None)).unwrap())
}

fn config() -> DreamScheduleConfig {
    DreamScheduleConfig {
        enabled: true,
        scope: MemoryScope::Global,
        mode: DreamMode::Refine,
        check_interval: Duration::seconds(1),
        stale_after: Duration::hours(6),
        lease_for: Duration::minutes(5),
        workers: 1,
        reasoner_model: None,
    }
}

#[tokio::test]
async fn scheduled_dream_skips_empty_scope() {
    let outcome = run_scheduled_dream_once(store(), config()).await.unwrap();
    assert_eq!(
        outcome,
        ScheduledDreamOutcome::Skipped {
            reason: ScheduledDreamSkipReason::NoFacts,
            scope_id: "global".to_string(),
        }
    );
}

#[tokio::test]
async fn scheduled_dream_runs_when_scope_has_no_recent_dream() {
    let store = store();
    store
        .capture(CaptureInput::new(
            MemoryScope::Global,
            "Retention policy is 90 days.",
        ))
        .await
        .unwrap();

    let outcome = run_scheduled_dream_once(store.clone(), config())
        .await
        .unwrap();
    let ScheduledDreamOutcome::Ran { run } = outcome else {
        panic!("expected scheduled dream to run");
    };
    assert_eq!(run.scope_id, "global");
    assert_eq!(run.mode, DreamMode::Refine);

    let second = run_scheduled_dream_once(store, config()).await.unwrap();
    assert_eq!(
        second,
        ScheduledDreamOutcome::Skipped {
            reason: ScheduledDreamSkipReason::Fresh,
            scope_id: "global".to_string(),
        }
    );
}

#[tokio::test]
async fn scheduled_dream_respects_disabled_config() {
    let mut disabled = config();
    disabled.enabled = false;
    let outcome = run_scheduled_dream_once(store(), disabled).await.unwrap();
    assert_eq!(
        outcome,
        ScheduledDreamOutcome::Skipped {
            reason: ScheduledDreamSkipReason::Disabled,
            scope_id: "global".to_string(),
        }
    );
}
