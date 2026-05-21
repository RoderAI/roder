use roder_api::automations::{
    AutomationClient, AutomationClientKind, AutomationConcurrencyPolicy, AutomationDefinition,
    AutomationProject, AutomationRunState, AutomationRunSummary, AutomationSchedule, CatchUpPolicy,
};
use roder_automations::AutomationSupervisorConfig;
use roder_automations::{
    AutomationStore, Clock, FakeClock, OccurrenceAction, RunLogEntry, expand_missed_occurrences,
    next_after, occurrence_key, run_due_tick,
};
use time::OffsetDateTime;

fn ts(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap()
}

fn parse(value: &str) -> OffsetDateTime {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).unwrap()
}

fn definition(schedule: AutomationSchedule, catch_up: CatchUpPolicy) -> AutomationDefinition {
    AutomationDefinition {
        id: "automation-1".to_string(),
        name: "Nightly status".to_string(),
        project: AutomationProject {
            cwd: "/tmp/project".to_string(),
            display_name: Some("project".to_string()),
        },
        schedule,
        prompt: "summarize status".to_string(),
        enabled: true,
        model_provider: Some("codex".to_string()),
        model: Some("gpt-5.5".to_string()),
        policy_mode: None,
        catch_up,
        concurrency: AutomationConcurrencyPolicy::Forbid,
        created_by: AutomationClient {
            id: "desktop-main".to_string(),
            kind: AutomationClientKind::Desktop,
        },
        created_at: ts(0),
        updated_at: ts(0),
    }
}

#[test]
fn schedule_interval_is_deterministic_with_fake_clock() {
    let clock = FakeClock::new(ts(125));
    let schedule = AutomationSchedule::Interval { seconds: 60 };

    assert_eq!(clock.now(), ts(125));
    assert_eq!(
        next_after(&schedule, clock.now(), false).unwrap(),
        Some(ts(180))
    );

    clock.set(ts(180));
    assert_eq!(
        next_after(&schedule, clock.now(), true).unwrap(),
        Some(ts(180))
    );
}

#[test]
fn schedule_cron_uses_timezone_across_dst_boundary() {
    let schedule = AutomationSchedule::Cron {
        expression: "30 2 * * *".to_string(),
        timezone: "Europe/London".to_string(),
    };

    let before_dst = next_after(&schedule, parse("2026-03-28T00:00:00Z"), false)
        .unwrap()
        .unwrap();
    let after_dst = next_after(&schedule, parse("2026-03-29T00:00:00Z"), false)
        .unwrap()
        .unwrap();

    assert_eq!(before_dst, parse("2026-03-28T02:30:00Z"));
    assert_eq!(after_dst, parse("2026-03-29T01:30:00Z"));
}

#[test]
fn schedule_one_shot_runs_once_inside_window() {
    let schedule = AutomationSchedule::OneShot { run_at: ts(120) };

    assert_eq!(
        next_after(&schedule, ts(100), false).unwrap(),
        Some(ts(120))
    );
    assert_eq!(next_after(&schedule, ts(120), false).unwrap(), None);
}

#[test]
fn scheduler_expands_startup_catch_up_with_cap() {
    let definition = definition(
        AutomationSchedule::Interval { seconds: 60 },
        CatchUpPolicy::RunAllMissed { max_per_tick: 3 },
    );

    let occurrences = expand_missed_occurrences(&definition, ts(0), ts(360)).unwrap();

    assert_eq!(occurrences.len(), 3);
    assert_eq!(occurrences[0].scheduled_for, ts(60));
    assert_eq!(occurrences[2].scheduled_for, ts(180));
    assert!(
        occurrences
            .iter()
            .all(|occurrence| occurrence.action == OccurrenceAction::Run)
    );
}

#[test]
fn scheduler_coalesces_to_latest_missed_occurrence() {
    let definition = definition(
        AutomationSchedule::Interval { seconds: 60 },
        CatchUpPolicy::RunLatestOnly,
    );

    let occurrences = expand_missed_occurrences(&definition, ts(0), ts(180)).unwrap();

    assert_eq!(occurrences.len(), 3);
    assert_eq!(
        occurrences[0].action,
        OccurrenceAction::Skip {
            reason: "coalesced_by_run_latest_only".to_string()
        }
    );
    assert_eq!(occurrences[2].scheduled_for, ts(180));
    assert_eq!(occurrences[2].action, OccurrenceAction::Run);
}

#[test]
fn scheduler_records_expired_missed_occurrences_as_skipped() {
    let definition = definition(
        AutomationSchedule::Interval { seconds: 60 },
        CatchUpPolicy::SkipExpired { grace_seconds: 70 },
    );

    let occurrences = expand_missed_occurrences(&definition, ts(0), ts(180)).unwrap();

    assert_eq!(occurrences.len(), 3);
    assert_eq!(
        occurrences[0].action,
        OccurrenceAction::Skip {
            reason: "expired_by_catch_up_grace".to_string()
        }
    );
    assert_eq!(occurrences[1].action, OccurrenceAction::Run);
    assert_eq!(occurrences[2].action, OccurrenceAction::Run);
}

#[test]
fn scheduler_ignores_disabled_automations() {
    let mut definition = definition(
        AutomationSchedule::Interval { seconds: 60 },
        CatchUpPolicy::RunAllMissed { max_per_tick: 10 },
    );
    definition.enabled = false;

    let occurrences = expand_missed_occurrences(&definition, ts(0), ts(180)).unwrap();

    assert!(occurrences.is_empty());
}

#[test]
fn store_persists_definitions_occurrences_runs_and_logs() {
    let store = AutomationStore::open_memory().unwrap();
    let definition = definition(
        AutomationSchedule::Interval { seconds: 60 },
        CatchUpPolicy::RunAllMissed { max_per_tick: 10 },
    );
    let occurrence = expand_missed_occurrences(&definition, ts(0), ts(60))
        .unwrap()
        .pop()
        .unwrap();

    store.upsert_automation(&definition, Some(ts(0))).unwrap();
    store.record_occurrence(&occurrence, ts(60)).unwrap();
    store.update_last_checked(&definition.id, ts(60)).unwrap();

    let stored = store.get_automation(&definition.id).unwrap().unwrap();
    assert_eq!(stored.definition, definition);
    assert_eq!(stored.last_checked_at, Some(ts(60)));

    let run = AutomationRunSummary {
        run_id: "run-1".to_string(),
        automation_id: definition.id.clone(),
        occurrence_key: occurrence.occurrence_key.clone(),
        state: AutomationRunState::Scheduled,
        scheduled_for: occurrence.scheduled_for,
        queued_at: None,
        started_at: None,
        finished_at: None,
        thread_id: None,
        turn_id: None,
        task_id: None,
        server_id: None,
        server_role: None,
        exit_code: None,
        error: None,
        skip_reason: None,
    };
    store.upsert_run(&run, ts(61)).unwrap();
    store
        .append_log(&RunLogEntry {
            run_id: run.run_id.clone(),
            stream: "log".to_string(),
            chunk: "queued".to_string(),
            timestamp: ts(61),
        })
        .unwrap();

    assert_eq!(store.get_run(&run.run_id).unwrap(), Some(run.clone()));
    let logs = store.list_logs(&run.run_id).unwrap();
    assert_eq!(logs[0].chunk, "queued");
}

#[test]
fn lease_prevents_duplicate_instances_and_recovers_stale_leases() {
    let store = AutomationStore::open_memory().unwrap();
    let definition = definition(
        AutomationSchedule::Interval { seconds: 60 },
        CatchUpPolicy::RunAllMissed { max_per_tick: 10 },
    );
    store.upsert_automation(&definition, Some(ts(0))).unwrap();
    let occurrence_key = occurrence_key(&definition.id, ts(60));

    let first = store
        .acquire_lease(
            "run-1".to_string(),
            definition.id.clone(),
            occurrence_key.clone(),
            "server-a".to_string(),
            "desktop".to_string(),
            ts(60),
            30,
        )
        .unwrap();
    assert!(first.is_some());

    let duplicate = store
        .acquire_lease(
            "run-2".to_string(),
            definition.id.clone(),
            occurrence_key.clone(),
            "server-b".to_string(),
            "desktop".to_string(),
            ts(70),
            30,
        )
        .unwrap();
    assert!(duplicate.is_none());

    let recovered = store
        .acquire_lease(
            "run-2".to_string(),
            definition.id.clone(),
            occurrence_key,
            "server-b".to_string(),
            "desktop".to_string(),
            ts(91),
            30,
        )
        .unwrap();
    assert!(recovered.is_some());
}

#[test]
fn lease_renew_and_release_require_owner() {
    let store = AutomationStore::open_memory().unwrap();
    let definition = definition(
        AutomationSchedule::Interval { seconds: 60 },
        CatchUpPolicy::RunAllMissed { max_per_tick: 10 },
    );
    store.upsert_automation(&definition, Some(ts(0))).unwrap();
    let run_id = "run-1".to_string();
    let server_id = "server-a".to_string();
    store
        .acquire_lease(
            run_id.clone(),
            definition.id.clone(),
            occurrence_key(&definition.id, ts(60)),
            server_id.clone(),
            "desktop".to_string(),
            ts(60),
            30,
        )
        .unwrap();

    assert!(
        !store
            .renew_lease(&run_id, &"server-b".to_string(), ts(70), 30)
            .unwrap()
    );
    assert!(store.renew_lease(&run_id, &server_id, ts(70), 30).unwrap());
    assert!(store.release_lease(&run_id).unwrap());
    assert!(!store.release_lease(&run_id).unwrap());
}

#[test]
fn scheduler_recovers_missed_cron_occurrences_after_process_restart() {
    let temp = tempfile::tempdir().unwrap();
    let store_path = temp.path().join("automations.sqlite3");
    let definition = definition(
        AutomationSchedule::Cron {
            expression: "* * * * *".to_string(),
            timezone: "UTC".to_string(),
        },
        CatchUpPolicy::RunAllMissed { max_per_tick: 10 },
    );
    let config = AutomationSupervisorConfig {
        enabled: true,
        max_due_per_tick: 10,
        ..AutomationSupervisorConfig::default()
    };

    {
        let store = AutomationStore::open(&store_path).unwrap();
        store.upsert_automation(&definition, Some(ts(0))).unwrap();
    }

    let restarted = AutomationStore::open(&store_path).unwrap();
    let clock = FakeClock::new(ts(300));
    let result = run_due_tick(&restarted, &config, &clock).unwrap();

    assert_eq!(result.occurrences_recorded, 5);
    assert_eq!(result.runs_due, 5);
    assert_eq!(
        restarted.count_occurrences_by_state("scheduled").unwrap(),
        5
    );
    let due = restarted.list_scheduled_occurrences(None).unwrap();
    assert_eq!(due.len(), 5);
    assert_eq!(due[0].1.scheduled_for, ts(60));
    assert_eq!(due[4].1.scheduled_for, ts(300));
}

#[test]
fn scheduler_racing_instances_do_not_lease_same_due_occurrence_twice() {
    let temp = tempfile::tempdir().unwrap();
    let store_path = temp.path().join("automations.sqlite3");
    let definition = definition(
        AutomationSchedule::Interval { seconds: 60 },
        CatchUpPolicy::RunAllMissed { max_per_tick: 10 },
    );
    let config = AutomationSupervisorConfig {
        enabled: true,
        max_due_per_tick: 1,
        ..AutomationSupervisorConfig::default()
    };
    let store_a = AutomationStore::open(&store_path).unwrap();
    store_a.upsert_automation(&definition, Some(ts(0))).unwrap();
    run_due_tick(&store_a, &config, &FakeClock::new(ts(60))).unwrap();

    let store_b = AutomationStore::open(&store_path).unwrap();
    let due_a = store_a.list_scheduled_occurrences(Some(1)).unwrap();
    let due_b = store_b.list_scheduled_occurrences(Some(1)).unwrap();
    assert_eq!(due_a[0].1.occurrence_key, due_b[0].1.occurrence_key);

    let first = store_a
        .acquire_lease(
            "run-a".to_string(),
            definition.id.clone(),
            due_a[0].1.occurrence_key.clone(),
            "server-a".to_string(),
            "desktop".to_string(),
            ts(60),
            30,
        )
        .unwrap();
    assert!(first.is_some());
    store_a
        .set_occurrence_state(&due_a[0].1.occurrence_key, "queued", None)
        .unwrap();

    let duplicate = store_b
        .acquire_lease(
            "run-b".to_string(),
            definition.id.clone(),
            due_b[0].1.occurrence_key.clone(),
            "server-b".to_string(),
            "desktop".to_string(),
            ts(61),
            30,
        )
        .unwrap();
    assert!(duplicate.is_none());
    assert_eq!(store_b.count_occurrences_by_state("queued").unwrap(), 1);
}

#[test]
fn scheduler_recovers_stale_leased_occurrences_after_lease_seconds() {
    let store = AutomationStore::open_memory().unwrap();
    let definition = definition(
        AutomationSchedule::Interval { seconds: 60 },
        CatchUpPolicy::RunAllMissed { max_per_tick: 10 },
    );
    let config = AutomationSupervisorConfig {
        enabled: true,
        lease_seconds: 30,
        ..AutomationSupervisorConfig::default()
    };
    store.upsert_automation(&definition, Some(ts(0))).unwrap();
    run_due_tick(&store, &config, &FakeClock::new(ts(60))).unwrap();
    let due = store.list_scheduled_occurrences(Some(1)).unwrap();
    let occurrence = &due[0].1;
    store
        .acquire_lease(
            "run-a".to_string(),
            definition.id.clone(),
            occurrence.occurrence_key.clone(),
            "server-a".to_string(),
            "desktop".to_string(),
            ts(60),
            30,
        )
        .unwrap();
    store
        .set_occurrence_state(&occurrence.occurrence_key, "queued", None)
        .unwrap();

    run_due_tick(&store, &config, &FakeClock::new(ts(91))).unwrap();

    assert_eq!(store.count_leases().unwrap(), 0);
    assert_eq!(store.count_occurrences_by_state("scheduled").unwrap(), 1);
    assert_eq!(
        store.list_scheduled_occurrences(Some(1)).unwrap()[0]
            .1
            .occurrence_key,
        occurrence.occurrence_key
    );
}

#[test]
fn scheduler_caps_due_per_tick_and_persists_skipped_occurrences() {
    let store = AutomationStore::open_memory().unwrap();
    let run_all = definition(
        AutomationSchedule::Interval { seconds: 60 },
        CatchUpPolicy::RunAllMissed { max_per_tick: 10 },
    );
    store.upsert_automation(&run_all, Some(ts(0))).unwrap();
    let capped = AutomationSupervisorConfig {
        enabled: true,
        max_due_per_tick: 2,
        ..AutomationSupervisorConfig::default()
    };
    let result = run_due_tick(&store, &capped, &FakeClock::new(ts(300))).unwrap();
    assert_eq!(result.occurrences_recorded, 2);
    assert_eq!(store.count_occurrences_by_state("scheduled").unwrap(), 2);

    let mut skipped = definition(
        AutomationSchedule::Interval { seconds: 60 },
        CatchUpPolicy::RunLatestOnly,
    );
    skipped.id = "automation-skip".to_string();
    store.upsert_automation(&skipped, Some(ts(0))).unwrap();
    let result = run_due_tick(&store, &capped, &FakeClock::new(ts(180))).unwrap();
    assert_eq!(result.skipped, 2);
    assert_eq!(store.count_occurrences_by_state("skipped").unwrap(), 2);
}
