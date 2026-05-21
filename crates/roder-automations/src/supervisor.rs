use std::path::PathBuf;

use anyhow::Result;
use time::OffsetDateTime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::clock::{Clock, SystemClock};
use crate::model::OccurrenceAction;
use crate::schedule::expand_missed_occurrences;
use crate::store::AutomationStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationSupervisorConfig {
    pub enabled: bool,
    pub server_id: String,
    pub server_role: String,
    pub store_path: PathBuf,
    pub tick_seconds: u64,
    pub lease_seconds: u64,
    pub max_due_per_tick: u32,
    pub run_missed_on_startup: bool,
    pub read_api_when_disabled: bool,
    pub allowed_project_roots: Vec<PathBuf>,
}

impl Default for AutomationSupervisorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_id: "desktop-main".to_string(),
            server_role: "desktop".to_string(),
            store_path: PathBuf::from("~/.roder/automations.sqlite3"),
            tick_seconds: 30,
            lease_seconds: 900,
            max_due_per_tick: 10,
            run_missed_on_startup: true,
            read_api_when_disabled: true,
            allowed_project_roots: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutomationTickResult {
    pub automations_checked: usize,
    pub occurrences_recorded: usize,
    pub runs_due: usize,
    pub skipped: usize,
}

pub struct AutomationSupervisorHandle {
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl AutomationSupervisorHandle {
    pub fn new(shutdown: oneshot::Sender<()>, join: JoinHandle<()>) -> Self {
        Self {
            shutdown: Some(shutdown),
            join: Some(join),
        }
    }

    pub async fn shutdown(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

impl Drop for AutomationSupervisorHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

pub fn start_supervisor(
    config: AutomationSupervisorConfig,
) -> Result<Option<AutomationSupervisorHandle>> {
    if !config.enabled {
        return Ok(None);
    }

    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    let store_path = config.store_path.clone();
    let join = tokio::spawn(async move {
        let Ok(store) = AutomationStore::open(store_path) else {
            return;
        };
        let clock = SystemClock;
        if config.run_missed_on_startup {
            let _ = run_due_tick(&store, &config, &clock);
        }
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(config.tick_seconds.max(1)));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let _ = run_due_tick(&store, &config, &clock);
                }
                _ = &mut shutdown_rx => break,
            }
        }
    });

    Ok(Some(AutomationSupervisorHandle {
        shutdown: Some(shutdown_tx),
        join: Some(join),
    }))
}

pub fn run_due_tick(
    store: &AutomationStore,
    config: &AutomationSupervisorConfig,
    clock: &dyn Clock,
) -> Result<AutomationTickResult> {
    let now = clock.now();
    let mut result = AutomationTickResult::default();
    let _ = store.recover_expired_leases(now)?;
    for stored in store.list_automations()? {
        result.automations_checked += 1;
        let last_checked_at = stored
            .last_checked_at
            .unwrap_or(stored.definition.created_at.min(now));
        let mut occurrences = expand_missed_occurrences(&stored.definition, last_checked_at, now)?;
        occurrences.truncate(config.max_due_per_tick as usize);
        for occurrence in &occurrences {
            store.record_occurrence(occurrence, now)?;
            result.occurrences_recorded += 1;
            match occurrence.action {
                OccurrenceAction::Run => result.runs_due += 1,
                OccurrenceAction::Skip { .. } => result.skipped += 1,
            }
        }
        store.update_last_checked(&stored.definition.id, now)?;
    }
    Ok(result)
}

pub fn tick_timestamp() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

#[cfg(test)]
mod tests {
    use roder_api::automations::{
        AutomationClient, AutomationClientKind, AutomationConcurrencyPolicy, AutomationDefinition,
        AutomationProject, AutomationSchedule, CatchUpPolicy,
    };

    use super::*;
    use crate::clock::FakeClock;

    fn ts(seconds: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(seconds).unwrap()
    }

    fn definition() -> AutomationDefinition {
        AutomationDefinition {
            id: "automation-1".to_string(),
            name: "Hourly status".to_string(),
            project: AutomationProject {
                cwd: "/tmp/project".to_string(),
                display_name: None,
            },
            schedule: AutomationSchedule::Interval { seconds: 60 },
            prompt: "summarize status".to_string(),
            enabled: true,
            model_provider: None,
            model: None,
            policy_mode: None,
            catch_up: CatchUpPolicy::RunAllMissed { max_per_tick: 10 },
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
    fn automations_supervisor_tick_records_due_work_off_request_path() {
        let store = AutomationStore::open_memory().unwrap();
        let definition = definition();
        store.upsert_automation(&definition, Some(ts(0))).unwrap();
        let config = AutomationSupervisorConfig {
            enabled: true,
            max_due_per_tick: 2,
            ..AutomationSupervisorConfig::default()
        };
        let clock = FakeClock::new(ts(180));

        let result = run_due_tick(&store, &config, &clock).unwrap();

        assert_eq!(result.automations_checked, 1);
        assert_eq!(result.occurrences_recorded, 2);
        assert_eq!(result.runs_due, 2);
        let stored = store.get_automation(&definition.id).unwrap().unwrap();
        assert_eq!(stored.last_checked_at, Some(ts(180)));
    }
}
