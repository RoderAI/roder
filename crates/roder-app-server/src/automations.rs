use std::path::PathBuf;
use std::sync::Arc;

use roder_api::automations::{
    AutomationClient, AutomationClientKind, AutomationDefinition, AutomationRunState,
    AutomationRunSummary,
};
use roder_automations::{AutomationStore, ScheduledOccurrence};
use roder_automations::{
    AutomationSupervisorConfig, AutomationSupervisorHandle, SystemClock, run_due_tick,
};
use roder_core::Runtime;
use roder_protocol::{
    AutomationsCancelRunParams, AutomationsCancelRunResult, AutomationsCreateParams,
    AutomationsCreateResult, AutomationsDeleteParams, AutomationsDeleteResult,
    AutomationsListParams, AutomationsListResult, AutomationsRunNowParams, AutomationsRunNowResult,
    AutomationsRunsParams, AutomationsRunsResult, AutomationsStatusResult, AutomationsUpdateParams,
    AutomationsUpdateResult, JsonRpcError,
};
use roder_tasks::{BackgroundRunner, BackgroundRunnerConfig, TaskExecutorRegistry};
use tokio::sync::{RwLock, broadcast, oneshot};

use crate::automation_worker::{
    AUTOMATION_TASK_EXECUTOR_ID, AutomationTaskExecutor, AutomationTaskInput,
};
use crate::notifications;
use crate::server::AppServer;
use crate::server::internal_error;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppServerFeatureConfig {
    pub automations: AutomationSupervisorConfig,
    pub workspace_registry_path: Option<PathBuf>,
}

impl AppServerFeatureConfig {
    pub fn from_config(config: Option<&roder_config::AppServerConfig>) -> Self {
        let Some(config) = config else {
            return Self::default();
        };
        Self {
            workspace_registry_path: None,
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

    pub fn with_workspace_registry_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace_registry_path = Some(path.into());
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
        let (protocol_notifications, _) = broadcast::channel(1024);
        if tokio::runtime::Handle::try_current().is_ok() {
            notifications::spawn_task_event_bridge(Arc::clone(&runtime), tasks.clone());
            notifications::spawn_runtime_event_handlers(Arc::clone(&runtime), tasks.clone());
            notifications::spawn_protocol_notification_bridge(
                Arc::clone(&runtime),
                protocol_notifications.clone(),
            );
        }
        let automation_supervisor = if tokio::runtime::Handle::try_current().is_ok() {
            start_app_server_automation_supervisor(
                tasks.clone(),
                feature_config.automations.clone(),
            )
            .ok()
            .flatten()
        } else {
            None
        };
        let workflows = crate::workflows::AppWorkflowService::new(Arc::clone(&runtime));
        let workspace_registry_path = feature_config
            .workspace_registry_path
            .clone()
            .unwrap_or_else(|| roder_config::config_dir().join("workspaces.json"));
        let workspace_files =
            crate::workspace_files::WorkspaceFileService::new(protocol_notifications.clone());
        Self {
            runtime,
            workflows,
            tasks,
            persist_user_config: false,
            features: feature_config,
            automation_supervisor,
            protocol_threads: RwLock::new(std::collections::HashMap::new()),
            protocol_default_model: RwLock::new(None),
            protocol_thread_models: RwLock::new(std::collections::HashMap::new()),
            protocol_notifications,
            workspaces: crate::workspaces::WorkspaceRegistry::new(workspace_registry_path),
            workspace_files,
            command_registry: tokio::sync::OnceCell::new(),
        }
    }

    pub fn with_automation_scheduler(
        runtime: Arc<Runtime>,
        automations: AutomationSupervisorConfig,
    ) -> Self {
        Self::with_feature_config(
            runtime,
            AppServerFeatureConfig {
                automations,
                ..AppServerFeatureConfig::default()
            },
        )
    }

    pub fn automation_status(&self) -> AutomationsStatusResult {
        let automations = &self.features.automations;
        let (active_runs, due_count, leased_count) = self
            .automation_store()
            .and_then(|store| {
                Ok((
                    store.count_runs_by_states(&[
                        AutomationRunState::Queued,
                        AutomationRunState::Running,
                    ])?,
                    store.count_occurrences_by_state("scheduled")?,
                    store.count_leases()?,
                ))
            })
            .unwrap_or((0, 0, 0));
        AutomationsStatusResult {
            scheduler_enabled: automations.enabled && self.automation_supervisor.is_some(),
            read_api_enabled: automations.enabled || automations.read_api_when_disabled,
            server_id: automations.server_id.clone(),
            server_role: automations.server_role.clone(),
            store_path: automations.store_path.display().to_string(),
            last_tick_at: None,
            next_tick_at: None,
            active_runs,
            due_count,
            leased_count,
        }
    }

    pub async fn submit_automation_run(
        &self,
        definition: AutomationDefinition,
        occurrence: ScheduledOccurrence,
    ) -> anyhow::Result<AutomationRunSummary> {
        submit_automation_run_with(
            &self.tasks,
            &self.features.automations,
            definition,
            occurrence,
        )
        .await
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

    pub(crate) async fn handle_automations_list(
        &self,
        params: AutomationsListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let automations = self
            .automation_store()
            .map_err(internal_error)?
            .list_automations()
            .map_err(internal_error)?
            .into_iter()
            .map(|stored| stored.definition)
            .filter(|definition| params.include_disabled.unwrap_or(true) || definition.enabled)
            .filter(|definition| {
                params
                    .project_cwd
                    .as_ref()
                    .is_none_or(|cwd| &definition.project.cwd == cwd)
            })
            .collect::<Vec<_>>();
        Ok(serde_json::to_value(AutomationsListResult { automations }).unwrap())
    }

    pub(crate) async fn handle_automations_create(
        &self,
        params: AutomationsCreateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        validate_project(&params.project.cwd).map_err(internal_error)?;
        validate_schedule(&params.schedule).map_err(internal_error)?;
        let now = time::OffsetDateTime::now_utc();
        let definition = AutomationDefinition {
            id: uuid::Uuid::new_v4().to_string(),
            name: params.name,
            project: params.project,
            schedule: params.schedule,
            prompt: params.prompt,
            enabled: params.enabled,
            model_provider: params.model_provider,
            model: params.model,
            policy_mode: params.policy_mode,
            catch_up: params.catch_up,
            concurrency: params.concurrency,
            created_by: AutomationClient {
                id: self.features.automations.server_id.clone(),
                kind: AutomationClientKind::AppServer,
            },
            created_at: now,
            updated_at: now,
        };
        self.automation_store()
            .map_err(internal_error)?
            .upsert_automation(&definition, Some(now))
            .map_err(internal_error)?;
        Ok(serde_json::to_value(AutomationsCreateResult {
            automation: definition,
        })
        .unwrap())
    }

    pub(crate) async fn handle_automations_update(
        &self,
        params: AutomationsUpdateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = self.automation_store().map_err(internal_error)?;
        let mut definition = store
            .get_automation(&params.automation_id)
            .map_err(internal_error)?
            .ok_or_else(|| unknown_automation(&params.automation_id))?
            .definition;
        if let Some(name) = params.patch.name {
            definition.name = name;
        }
        if let Some(project) = params.patch.project {
            validate_project(&project.cwd).map_err(internal_error)?;
            definition.project = project;
        }
        if let Some(schedule) = params.patch.schedule {
            validate_schedule(&schedule).map_err(internal_error)?;
            definition.schedule = schedule;
        }
        if let Some(prompt) = params.patch.prompt {
            definition.prompt = prompt;
        }
        if let Some(enabled) = params.patch.enabled {
            definition.enabled = enabled;
        }
        if let Some(model_provider) = params.patch.model_provider {
            definition.model_provider = Some(model_provider);
        }
        if let Some(model) = params.patch.model {
            definition.model = Some(model);
        }
        if let Some(policy_mode) = params.patch.policy_mode {
            definition.policy_mode = Some(policy_mode);
        }
        if let Some(catch_up) = params.patch.catch_up {
            definition.catch_up = catch_up;
        }
        if let Some(concurrency) = params.patch.concurrency {
            definition.concurrency = concurrency;
        }
        definition.updated_at = time::OffsetDateTime::now_utc();
        store
            .upsert_automation(&definition, Some(definition.updated_at))
            .map_err(internal_error)?;
        Ok(serde_json::to_value(AutomationsUpdateResult {
            automation: definition,
        })
        .unwrap())
    }

    pub(crate) async fn handle_automations_delete(
        &self,
        params: AutomationsDeleteParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = self.automation_store().map_err(internal_error)?;
        let Some(mut stored) = store
            .get_automation(&params.automation_id)
            .map_err(internal_error)?
        else {
            return Ok(serde_json::to_value(AutomationsDeleteResult {
                automation_id: params.automation_id,
                deleted: false,
            })
            .unwrap());
        };
        stored.definition.enabled = false;
        stored.definition.updated_at = time::OffsetDateTime::now_utc();
        store
            .upsert_automation(&stored.definition, stored.last_checked_at)
            .map_err(internal_error)?;
        Ok(serde_json::to_value(AutomationsDeleteResult {
            automation_id: params.automation_id,
            deleted: true,
        })
        .unwrap())
    }

    pub(crate) async fn handle_automations_run_now(
        &self,
        params: AutomationsRunNowParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut definition = self
            .automation_store()
            .map_err(internal_error)?
            .get_automation(&params.automation_id)
            .map_err(internal_error)?
            .ok_or_else(|| unknown_automation(&params.automation_id))?
            .definition;
        if let Some(prompt) = params.prompt_override {
            definition.prompt = prompt;
        }
        let scheduled_for = time::OffsetDateTime::now_utc();
        let occurrence = ScheduledOccurrence {
            automation_id: definition.id.clone(),
            occurrence_key: roder_automations::occurrence_key(&definition.id, scheduled_for),
            scheduled_for,
            action: roder_automations::OccurrenceAction::Run,
        };
        let run = self
            .submit_automation_run(definition, occurrence)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(AutomationsRunNowResult { run }).unwrap())
    }

    pub(crate) async fn handle_automations_runs(
        &self,
        params: AutomationsRunsParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut runs = self
            .automation_store()
            .map_err(internal_error)?
            .list_runs(&params.automation_id, params.limit)
            .map_err(internal_error)?;
        if let Some(state) = params.state {
            runs.retain(|run| run.state == state);
        }
        Ok(serde_json::to_value(AutomationsRunsResult {
            runs,
            next_cursor: None,
        })
        .unwrap())
    }

    pub(crate) async fn handle_automations_cancel_run(
        &self,
        params: AutomationsCancelRunParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cancelled = self
            .cancel_automation_run(&params.run_id, params.reason)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(AutomationsCancelRunResult {
            run_id: params.run_id,
            cancelled,
        })
        .unwrap())
    }

    pub(crate) async fn handle_automations_status(
        &self,
    ) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(self.automation_status()).unwrap())
    }

    fn automation_store(&self) -> anyhow::Result<AutomationStore> {
        AutomationStore::open(&self.features.automations.store_path)
    }
}

fn start_app_server_automation_supervisor(
    tasks: BackgroundRunner,
    config: AutomationSupervisorConfig,
) -> anyhow::Result<Option<AutomationSupervisorHandle>> {
    if !config.enabled {
        return Ok(None);
    }

    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(async move {
        let Ok(store) = AutomationStore::open(&config.store_path) else {
            return;
        };
        let clock = SystemClock;
        if config.run_missed_on_startup {
            let _ = run_due_tick(&store, &config, &clock);
            drain_due_automation_runs(&tasks, &config).await;
        }
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(config.tick_seconds.max(1)));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let _ = run_due_tick(&store, &config, &clock);
                    drain_due_automation_runs(&tasks, &config).await;
                }
                _ = &mut shutdown_rx => break,
            }
        }
    });

    Ok(Some(AutomationSupervisorHandle::new(shutdown_tx, join)))
}

async fn drain_due_automation_runs(tasks: &BackgroundRunner, config: &AutomationSupervisorConfig) {
    let Ok(store) = AutomationStore::open(&config.store_path) else {
        return;
    };
    let Ok(due) = store.list_scheduled_occurrences(Some(config.max_due_per_tick as usize)) else {
        return;
    };
    for (definition, occurrence) in due {
        let _ = submit_automation_run_with(tasks, config, definition, occurrence).await;
    }
}

async fn submit_automation_run_with(
    tasks: &BackgroundRunner,
    config: &AutomationSupervisorConfig,
    definition: AutomationDefinition,
    occurrence: ScheduledOccurrence,
) -> anyhow::Result<AutomationRunSummary> {
    let store = AutomationStore::open(&config.store_path)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let lease = store
        .acquire_lease(
            run_id.clone(),
            definition.id.clone(),
            occurrence.occurrence_key.clone(),
            config.server_id.clone(),
            config.server_role.clone(),
            time::OffsetDateTime::now_utc(),
            config.lease_seconds,
        )?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "automation occurrence {:?} is already leased",
                occurrence.occurrence_key
            )
        })?;
    if !store.set_occurrence_state(&occurrence.occurrence_key, "queued", None)? {
        store.record_occurrence(&occurrence, time::OffsetDateTime::now_utc())?;
        let _ = store.set_occurrence_state(&occurrence.occurrence_key, "queued", None)?;
    }
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
    let task = tasks
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

fn validate_project(cwd: &str) -> anyhow::Result<()> {
    if !std::path::Path::new(cwd).is_dir() {
        anyhow::bail!("automation project path does not exist: {cwd}");
    }
    Ok(())
}

fn validate_schedule(schedule: &roder_api::automations::AutomationSchedule) -> anyhow::Result<()> {
    roder_automations::next_after(schedule, time::OffsetDateTime::now_utc(), true)?;
    Ok(())
}

fn unknown_automation(id: &str) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("unknown automation {id:?}"),
        data: None,
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
