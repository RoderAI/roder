use std::path::PathBuf;
use std::sync::Arc;

use roder_api::automations::{AutomationDefinition, AutomationRunState, AutomationRunSummary};
use roder_automations::{AutomationStore, ScheduledOccurrence};
use roder_automations::{AutomationSupervisorConfig, start_supervisor};
use roder_core::Runtime;
use roder_protocol::AutomationsStatusResult;
use roder_tasks::{BackgroundRunner, BackgroundRunnerConfig, TaskExecutorRegistry};
use tokio::sync::{RwLock, broadcast};

use crate::automation_worker::{
    AUTOMATION_TASK_EXECUTOR_ID, AutomationTaskExecutor, AutomationTaskInput,
};
use crate::notifications;
use crate::server::AppServer;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppServerFeatureConfig {
    pub automations: AutomationSupervisorConfig,
}

impl AppServerFeatureConfig {
    pub fn from_config(config: Option<&roder_config::AppServerConfig>) -> Self {
        let Some(config) = config else {
            return Self::default();
        };
        Self {
            automations: AutomationSupervisorConfig {
                enabled: config.automations.enabled,
                server_id: config.automations.server_id.clone(),
                server_role: config.automations.server_role.clone(),
                store_path: config.automations.store_path.clone(),
                tick_seconds: config.automations.tick_seconds,
                lease_seconds: config.automations.lease_seconds,
                max_due_per_tick: config.automations.max_due_per_tick,
                run_missed_on_startup: config.automations.run_missed_on_startup,
                read_api_when_disabled: config.automations.read_api_when_disabled,
                allowed_project_roots: config.automations.allowed_project_roots.clone(),
            },
        }
    }

    pub fn with_automations_enabled(mut self, enabled: bool) -> Self {
        self.automations.enabled = enabled;
        self
    }

    pub fn with_automation_server_id(mut self, server_id: impl Into<String>) -> Self {
        self.automations.server_id = server_id.into();
        self
    }

    pub fn with_automation_server_role(mut self, server_role: impl Into<String>) -> Self {
        self.automations.server_role = server_role.into();
        self
    }

    pub fn with_automation_store_path(mut self, store_path: impl Into<PathBuf>) -> Self {
        self.automations.store_path = store_path.into();
        self
    }
}

impl AppServer {
    pub fn with_feature_config(
        runtime: Arc<Runtime>,
        feature_config: AppServerFeatureConfig,
    ) -> Self {
        let mut task_registry = TaskExecutorRegistry::default();
        for executor in &runtime.registry().task_executors {
            let _ = task_registry.register(Arc::clone(executor));
        }
        let _ = task_registry.register(Arc::new(AutomationTaskExecutor::new(
            Arc::clone(&runtime),
            feature_config.automations.clone(),
        )));
        let tasks = BackgroundRunner::new(task_registry, BackgroundRunnerConfig::default());
        let (desktop_notifications, _) = broadcast::channel(1024);
        if tokio::runtime::Handle::try_current().is_ok() {
            notifications::spawn_task_event_bridge(Arc::clone(&runtime), tasks.clone());
            notifications::spawn_runtime_event_handlers(Arc::clone(&runtime), tasks.clone());
            notifications::spawn_desktop_notification_bridge(
                Arc::clone(&runtime),
                desktop_notifications.clone(),
            );
        }
        let automation_supervisor = if tokio::runtime::Handle::try_current().is_ok() {
            start_supervisor(feature_config.automations.clone())
                .ok()
                .flatten()
        } else {
            None
        };
        Self {
            runtime,
            tasks,
            persist_user_config: false,
            features: feature_config,
            automation_supervisor,
            desktop_threads: RwLock::new(std::collections::HashMap::new()),
            desktop_thread_models: RwLock::new(std::collections::HashMap::new()),
            desktop_active_turns: RwLock::new(std::collections::HashMap::new()),
            desktop_notifications,
        }
    }

    pub fn with_automation_scheduler(
        runtime: Arc<Runtime>,
        automations: AutomationSupervisorConfig,
    ) -> Self {
        Self::with_feature_config(runtime, AppServerFeatureConfig { automations })
    }

    pub fn automation_status(&self) -> AutomationsStatusResult {
        let automations = &self.features.automations;
        AutomationsStatusResult {
            scheduler_enabled: automations.enabled && self.automation_supervisor.is_some(),
            read_api_enabled: automations.enabled || automations.read_api_when_disabled,
            server_id: automations.server_id.clone(),
            server_role: automations.server_role.clone(),
            store_path: automations.store_path.display().to_string(),
            last_tick_at: None,
            next_tick_at: None,
            active_runs: 0,
        }
    }

    pub async fn submit_automation_run(
        &self,
        definition: AutomationDefinition,
        occurrence: ScheduledOccurrence,
    ) -> anyhow::Result<AutomationRunSummary> {
        let store = AutomationStore::open(&self.features.automations.store_path)?;
        store.upsert_automation(&definition, Some(occurrence.scheduled_for))?;
        store.record_occurrence(&occurrence, time::OffsetDateTime::now_utc())?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let lease = store
            .acquire_lease(
                run_id.clone(),
                definition.id.clone(),
                occurrence.occurrence_key.clone(),
                self.features.automations.server_id.clone(),
                self.features.automations.server_role.clone(),
                time::OffsetDateTime::now_utc(),
                self.features.automations.lease_seconds,
            )?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "automation occurrence {:?} is already leased",
                    occurrence.occurrence_key
                )
            })?;
        let queued = AutomationRunSummary {
            run_id: run_id.clone(),
            automation_id: definition.id.clone(),
            occurrence_key: occurrence.occurrence_key.clone(),
            state: AutomationRunState::Queued,
            scheduled_for: occurrence.scheduled_for,
            queued_at: Some(time::OffsetDateTime::now_utc()),
            started_at: None,
            finished_at: None,
            thread_id: None,
            turn_id: None,
            task_id: None,
            server_id: Some(lease.server_id),
            server_role: Some(lease.server_role),
            exit_code: None,
            error: None,
            skip_reason: None,
        };
        store.upsert_run(&queued, time::OffsetDateTime::now_utc())?;
        let task = self
            .tasks
            .submit(
                AUTOMATION_TASK_EXECUTOR_ID,
                serde_json::to_value(AutomationTaskInput {
                    definition,
                    occurrence,
                    run_id: run_id.clone(),
                })?,
                roder_tasks::TaskSubmitOptions {
                    metadata: serde_json::json!({
                        "automationId": queued.automation_id,
                        "automationRunId": run_id,
                    }),
                    ..roder_tasks::TaskSubmitOptions::default()
                },
            )
            .await?;
        let queued = AutomationRunSummary {
            task_id: Some(task.task_id),
            ..queued
        };
        store.upsert_run(&queued, time::OffsetDateTime::now_utc())?;
        Ok(queued)
    }

    pub async fn cancel_automation_run(
        &self,
        run_id: &str,
        reason: Option<String>,
    ) -> anyhow::Result<bool> {
        let store = AutomationStore::open(&self.features.automations.store_path)?;
        let Some(run) = store.get_run(&run_id.to_string())? else {
            return Ok(false);
        };
        let task_cancelled = if let Some(task_id) = run.task_id.as_deref() {
            self.tasks
                .cancel(task_id, reason.clone())
                .await
                .unwrap_or(false)
        } else {
            false
        };
        let cancelled = AutomationRunSummary {
            state: AutomationRunState::Cancelled,
            finished_at: Some(time::OffsetDateTime::now_utc()),
            error: reason,
            ..run
        };
        store.upsert_run(&cancelled, time::OffsetDateTime::now_utc())?;
        let _ = store.release_lease(&cancelled.run_id);
        Ok(task_cancelled || cancelled.task_id.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automations_supervisor_config_is_disabled_by_default() {
        let config = AppServerFeatureConfig::default();

        assert!(!config.automations.enabled);
        assert_eq!(config.automations.server_id, "desktop-main");
        assert_eq!(config.automations.server_role, "desktop");
        assert!(config.automations.read_api_when_disabled);
    }

    #[test]
    fn automations_supervisor_config_uses_roder_config_values() {
        let config = roder_config::AppServerConfig {
            automations: roder_config::AppServerAutomationsConfig {
                enabled: true,
                server_id: "server-a".to_string(),
                server_role: "desktop".to_string(),
                store_path: PathBuf::from("/tmp/automations.sqlite3"),
                tick_seconds: 5,
                lease_seconds: 30,
                max_due_per_tick: 2,
                run_missed_on_startup: false,
                read_api_when_disabled: true,
                allowed_project_roots: vec![PathBuf::from("/repo")],
            },
        };

        let resolved = AppServerFeatureConfig::from_config(Some(&config));

        assert!(resolved.automations.enabled);
        assert_eq!(resolved.automations.server_id, "server-a");
        assert_eq!(
            resolved.automations.store_path,
            PathBuf::from("/tmp/automations.sqlite3")
        );
        assert_eq!(resolved.automations.max_due_per_tick, 2);
        assert_eq!(
            resolved.automations.allowed_project_roots,
            vec![PathBuf::from("/repo")]
        );
    }
}
