//! Periodic at-rest dream scheduling.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use roder_api::memory::MemoryScope;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use tokio::task::JoinHandle;

use crate::dream::{DreamMode, DreamPolicy};
use crate::model::{format_time, parse_time};
use crate::store::{DreamParams, DreamRunReport, GbrainStore};

#[derive(Debug, Clone)]
pub struct DreamScheduleConfig {
    pub enabled: bool,
    pub scope: MemoryScope,
    pub mode: DreamMode,
    pub check_interval: Duration,
    pub stale_after: Duration,
    pub lease_for: Duration,
    pub workers: usize,
    pub reasoner_model: Option<String>,
}

impl Default for DreamScheduleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scope: MemoryScope::Global,
            mode: DreamMode::Refine,
            check_interval: Duration::hours(1),
            stale_after: Duration::hours(6),
            lease_for: Duration::minutes(30),
            workers: 1,
            reasoner_model: None,
        }
    }
}

impl DreamScheduleConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if env_bool("RODER_GBRAIN_DREAM_SCHEDULER") == Some(false) {
            config.enabled = false;
        }
        if let Some(seconds) = env_seconds("RODER_GBRAIN_DREAM_CHECK_SECONDS") {
            config.check_interval = Duration::seconds(seconds.max(1));
        }
        if let Some(seconds) = env_seconds("RODER_GBRAIN_DREAM_STALE_SECONDS") {
            config.stale_after = Duration::seconds(seconds.max(1));
        }
        if let Some(seconds) = env_seconds("RODER_GBRAIN_DREAM_LEASE_SECONDS") {
            config.lease_for = Duration::seconds(seconds.max(1));
        }
        if let Ok(workers) = std::env::var("RODER_GBRAIN_DREAM_WORKERS")
            && let Ok(workers) = workers.parse::<usize>()
        {
            config.workers = workers.max(1);
        }
        if let Ok(model) = std::env::var("RODER_GBRAIN_DREAM_REASONER_MODEL")
            && !model.trim().is_empty()
        {
            config.reasoner_model = Some(model);
        }
        config
    }

    fn std_check_interval(&self) -> StdDuration {
        self.check_interval
            .try_into()
            .unwrap_or_else(|_| StdDuration::from_secs(3600))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledDreamSkipReason {
    Disabled,
    NoFacts,
    Fresh,
    Leased,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledDreamOutcome {
    Skipped {
        reason: ScheduledDreamSkipReason,
        scope_id: String,
    },
    Ran {
        run: DreamRunReport,
    },
}

/// Spawn the periodic at-rest scheduler for a running Roder process. If the
/// extension is installed outside an async runtime, this degrades to no-op.
pub fn spawn_periodic_dream_scheduler(
    store: Arc<GbrainStore>,
    config: DreamScheduleConfig,
) -> Option<JoinHandle<()>> {
    if !config.enabled {
        return None;
    }
    let handle = tokio::runtime::Handle::try_current().ok()?;
    Some(handle.spawn(async move {
        let mut interval = tokio::time::interval(config.std_check_interval());
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let _ = run_scheduled_dream_once(store.clone(), config.clone()).await;
        }
    }))
}

pub async fn run_scheduled_dream_once(
    store: Arc<GbrainStore>,
    config: DreamScheduleConfig,
) -> anyhow::Result<ScheduledDreamOutcome> {
    let scope_id = config.scope.stable_id();
    if !config.enabled {
        return Ok(ScheduledDreamOutcome::Skipped {
            reason: ScheduledDreamSkipReason::Disabled,
            scope_id,
        });
    }

    let now = OffsetDateTime::now_utc();
    match acquire_due_lease(&store, &config, now)? {
        LeaseDecision::Run => {}
        LeaseDecision::Skip(reason) => {
            return Ok(ScheduledDreamOutcome::Skipped { reason, scope_id });
        }
    }

    let run = store
        .dream(DreamParams {
            mode: config.mode,
            scope: config.scope.clone(),
            since: None,
            run_policy: DreamPolicy::Maintenance,
            workers: config.workers,
            dry_run: false,
            cancellation_token: None,
            reasoner_model: config.reasoner_model.clone(),
        })
        .await;

    match run {
        Ok(run) => {
            release_lease(&store, &config.scope, Some(&run.id))?;
            Ok(ScheduledDreamOutcome::Ran { run })
        }
        Err(err) => {
            let _ = release_lease(&store, &config.scope, None);
            Err(err)
        }
    }
}

enum LeaseDecision {
    Run,
    Skip(ScheduledDreamSkipReason),
}

fn acquire_due_lease(
    store: &GbrainStore,
    config: &DreamScheduleConfig,
    now: OffsetDateTime,
) -> anyhow::Result<LeaseDecision> {
    store.with_conn(|conn| {
        let scope_id = crate::store::ensure_scope(conn, &config.scope)?;
        let fact_count = crate::store::count_facts_since(conn, &config.scope, None)?;
        if fact_count == 0 {
            return Ok(LeaseDecision::Skip(ScheduledDreamSkipReason::NoFacts));
        }

        let last_finished: Option<String> = conn
            .query_row(
                "SELECT finished_at
                 FROM gbrain_dream_runs
                 WHERE scope_id = ?1 AND status = 'completed' AND finished_at IS NOT NULL
                 ORDER BY finished_at DESC
                 LIMIT 1",
                rusqlite::params![scope_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(last_finished) = last_finished
            && let Ok(last_finished) = parse_time(&last_finished)
            && now - last_finished < config.stale_after
        {
            return Ok(LeaseDecision::Skip(ScheduledDreamSkipReason::Fresh));
        }

        conn.execute(
            "INSERT OR IGNORE INTO gbrain_dream_schedule_leases(scope_id, updated_at)
             VALUES (?1, ?2)",
            rusqlite::params![scope_id, format_time(now)],
        )?;
        let lease_until = now + config.lease_for;
        let owner = format!("roder-session:{}", uuid::Uuid::new_v4());
        let updated = conn.execute(
            "UPDATE gbrain_dream_schedule_leases
             SET lease_owner = ?1, lease_until = ?2, last_checked_at = ?3, updated_at = ?3
             WHERE scope_id = ?4 AND (lease_until IS NULL OR lease_until <= ?3)",
            rusqlite::params![owner, format_time(lease_until), format_time(now), scope_id,],
        )?;
        if updated == 0 {
            return Ok(LeaseDecision::Skip(ScheduledDreamSkipReason::Leased));
        }
        Ok(LeaseDecision::Run)
    })
}

fn release_lease(
    store: &GbrainStore,
    scope: &MemoryScope,
    run_id: Option<&str>,
) -> anyhow::Result<()> {
    store.with_conn(|conn| {
        let scope_id = scope.stable_id();
        conn.execute(
            "UPDATE gbrain_dream_schedule_leases
             SET lease_owner = NULL, lease_until = NULL, last_scheduled_run_id = ?1, updated_at = ?2
             WHERE scope_id = ?3",
            rusqlite::params![run_id, format_time(OffsetDateTime::now_utc()), scope_id],
        )?;
        Ok(())
    })
}

fn env_seconds(name: &str) -> Option<i64> {
    std::env::var(name).ok()?.parse().ok()
}

fn env_bool(name: &str) -> Option<bool> {
    let value = std::env::var(name).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "0" | "false" | "off" | "no" => Some(false),
        "1" | "true" | "on" | "yes" => Some(true),
        _ => None,
    }
}
