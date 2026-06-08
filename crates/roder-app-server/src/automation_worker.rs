use std::sync::Arc;

use roder_api::automations::{
    AutomationCompleted, AutomationDefinition, AutomationFailed, AutomationRunState,
    AutomationRunSummary, AutomationStarted,
};
use roder_api::events::RoderEvent;
use roder_api::extension::TaskExecutorId;
use roder_api::tasks::{
    TaskExecutionContext, TaskExecutionResult, TaskExecutor, TaskOutputStream, TaskSpec,
};
use roder_automations::{AutomationStore, AutomationSupervisorConfig, ScheduledOccurrence};
use roder_core::{CreateThreadRequest, Runtime, StartTurnRequest, default_instructions};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub const AUTOMATION_TASK_EXECUTOR_ID: &str = "automation.run";

#[derive(Clone)]
pub struct AutomationTaskExecutor {
    runtime: Arc<Runtime>,
    config: AutomationSupervisorConfig,
}

impl AutomationTaskExecutor {
    pub fn new(runtime: Arc<Runtime>, config: AutomationSupervisorConfig) -> Self {
        Self { runtime, config }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationTaskInput {
    pub definition: AutomationDefinition,
    pub occurrence: ScheduledOccurrence,
    pub run_id: String,
}

#[async_trait::async_trait]
impl TaskExecutor for AutomationTaskExecutor {
    fn id(&self) -> TaskExecutorId {
        AUTOMATION_TASK_EXECUTOR_ID.to_string()
    }

    fn spec(&self) -> TaskSpec {
        TaskSpec {
            kind: AUTOMATION_TASK_EXECUTOR_ID.to_string(),
            description: "Execute a scheduled Roder automation run".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["definition", "occurrence", "runId"],
                "properties": {
                    "definition": { "type": "object" },
                    "occurrence": { "type": "object" },
                    "runId": { "type": "string" }
                }
            }),
            default_timeout_seconds: None,
            metadata: serde_json::json!({ "automation": true }),
        }
    }

    async fn execute(
        &self,
        ctx: TaskExecutionContext,
        input: serde_json::Value,
    ) -> anyhow::Result<TaskExecutionResult> {
        let input: AutomationTaskInput = serde_json::from_value(input)?;
        execute_automation_task(&self.runtime, &self.config, ctx, input).await
    }
}

pub async fn execute_automation_task(
    runtime: &Arc<Runtime>,
    config: &AutomationSupervisorConfig,
    ctx: TaskExecutionContext,
    input: AutomationTaskInput,
) -> anyhow::Result<TaskExecutionResult> {
    let store = AutomationStore::open(&config.store_path)?;
    let now = OffsetDateTime::now_utc();
    if !std::path::Path::new(&input.definition.project.cwd).is_dir() {
        let run = run_summary(
            &input,
            AutomationRunState::Failed,
            Some(ctx.task_id.clone()),
            None,
            None,
            Some(format!(
                "automation project path does not exist: {}",
                input.definition.project.cwd
            )),
        );
        store.upsert_run(&run, now)?;
        return Ok(TaskExecutionResult {
            exit_code: Some(1),
            payload: serde_json::to_value(run)?,
        });
    }

    ctx.output
        .write(
            TaskOutputStream::Log,
            format!("starting automation {}\n", input.definition.id),
        )
        .await?;

    let thread = runtime
        .create_thread_with(CreateThreadRequest {
            title: Some(format!("Automation: {}", input.definition.name)),
            workspace: input.definition.project.cwd.clone(),
            workspace_id: None,
            root_id: None,
            provider: input.definition.model_provider.clone(),
            model: input.definition.model.clone(),
            selection_mode: None,
        })
        .await?;
    let mut events = runtime.subscribe_events();
    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: thread.thread_id.clone(),
            message: input.definition.prompt.clone(),
            images: Vec::new(),
            provider_override: input.definition.model_provider.clone(),
            model_override: input.definition.model.clone(),
            reasoning_override: None,
            workspace: input.definition.project.cwd.clone(),
            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await?;

    let running = run_summary(
        &input,
        AutomationRunState::Running,
        Some(ctx.task_id.clone()),
        Some(thread.thread_id.clone()),
        Some(turn_id.clone()),
        None,
    );
    store.upsert_run(&running, OffsetDateTime::now_utc())?;
    runtime
        .emit(RoderEvent::AutomationStarted(AutomationStarted {
            run: running.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;

    loop {
        let event = events.recv().await?;
        if event.thread_id.as_deref() != Some(&thread.thread_id)
            || event.turn_id.as_deref() != Some(&turn_id)
        {
            continue;
        }
        match event.event {
            RoderEvent::TurnCompleted(_) => {
                let completed = run_summary(
                    &input,
                    AutomationRunState::Completed,
                    Some(ctx.task_id.clone()),
                    Some(thread.thread_id.clone()),
                    Some(turn_id.clone()),
                    None,
                );
                store.upsert_run(&completed, OffsetDateTime::now_utc())?;
                runtime
                    .emit(RoderEvent::AutomationCompleted(AutomationCompleted {
                        run: completed.clone(),
                        timestamp: OffsetDateTime::now_utc(),
                    }))
                    .await;
                let _ = store.release_lease(&input.run_id);
                return Ok(TaskExecutionResult::success(serde_json::to_value(
                    completed,
                )?));
            }
            RoderEvent::TurnFailed(failed) => {
                let failed_run = run_summary(
                    &input,
                    AutomationRunState::Failed,
                    Some(ctx.task_id.clone()),
                    Some(thread.thread_id.clone()),
                    Some(turn_id.clone()),
                    Some(failed.error.clone()),
                );
                store.upsert_run(&failed_run, OffsetDateTime::now_utc())?;
                runtime
                    .emit(RoderEvent::AutomationFailed(AutomationFailed {
                        run: failed_run.clone(),
                        error: failed.error,
                        timestamp: OffsetDateTime::now_utc(),
                    }))
                    .await;
                let _ = store.release_lease(&input.run_id);
                return Ok(TaskExecutionResult {
                    exit_code: Some(1),
                    payload: serde_json::to_value(failed_run)?,
                });
            }
            RoderEvent::ApprovalRequested(_) | RoderEvent::UserInputRequested(_) => {
                let error = "automation run blocked waiting for interactive input".to_string();
                let _ = runtime
                    .interrupt_turn(thread.thread_id.clone(), turn_id.clone())
                    .await;
                let failed_run = run_summary(
                    &input,
                    AutomationRunState::Failed,
                    Some(ctx.task_id.clone()),
                    Some(thread.thread_id.clone()),
                    Some(turn_id.clone()),
                    Some(error.clone()),
                );
                store.upsert_run(&failed_run, OffsetDateTime::now_utc())?;
                runtime
                    .emit(RoderEvent::AutomationFailed(AutomationFailed {
                        run: failed_run.clone(),
                        error,
                        timestamp: OffsetDateTime::now_utc(),
                    }))
                    .await;
                let _ = store.release_lease(&input.run_id);
                return Ok(TaskExecutionResult {
                    exit_code: Some(1),
                    payload: serde_json::to_value(failed_run)?,
                });
            }
            _ => {}
        }
    }
}

fn run_summary(
    input: &AutomationTaskInput,
    state: AutomationRunState,
    task_id: Option<String>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    error: Option<String>,
) -> AutomationRunSummary {
    AutomationRunSummary {
        run_id: input.run_id.clone(),
        automation_id: input.definition.id.clone(),
        occurrence_key: input.occurrence.occurrence_key.clone(),
        state,
        scheduled_for: input.occurrence.scheduled_for,
        queued_at: None,
        started_at: matches!(state, AutomationRunState::Running).then(OffsetDateTime::now_utc),
        finished_at: matches!(
            state,
            AutomationRunState::Completed
                | AutomationRunState::Failed
                | AutomationRunState::Skipped
        )
        .then(OffsetDateTime::now_utc),
        thread_id,
        turn_id,
        task_id,
        server_id: Some("automation-worker".to_string()),
        server_role: Some("app-server".to_string()),
        exit_code: matches!(state, AutomationRunState::Completed).then_some(0),
        error,
        skip_reason: None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use roder_api::automations::{
        AutomationClient, AutomationClientKind, AutomationConcurrencyPolicy, AutomationProject,
        AutomationSchedule, CatchUpPolicy,
    };
    use roder_automations::{OccurrenceAction, ScheduledOccurrence, occurrence_key};

    use super::*;
    use crate::{AppServer, AppServerFeatureConfig};

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "roder-automation-worker-{name}-{}",
            std::process::id()
        ))
    }

    fn definition(project: PathBuf) -> AutomationDefinition {
        AutomationDefinition {
            id: "automation-1".to_string(),
            name: "Mock automation".to_string(),
            project: AutomationProject {
                cwd: project.display().to_string(),
                display_name: None,
            },
            schedule: AutomationSchedule::Interval { seconds: 60 },
            prompt: "say hello".to_string(),
            enabled: true,
            model_provider: Some("mock".to_string()),
            model: Some("mock".to_string()),
            policy_mode: None,
            catch_up: CatchUpPolicy::RunAllMissed { max_per_tick: 10 },
            concurrency: AutomationConcurrencyPolicy::Forbid,
            created_by: AutomationClient {
                id: "test".to_string(),
                kind: AutomationClientKind::System,
            },
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn occurrence(definition: &AutomationDefinition) -> ScheduledOccurrence {
        let scheduled_for = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(60);
        ScheduledOccurrence {
            automation_id: definition.id.clone(),
            occurrence_key: occurrence_key(&definition.id, scheduled_for),
            scheduled_for,
            action: OccurrenceAction::Run,
        }
    }

    #[tokio::test]
    async fn automation_worker_runs_mock_turn_and_records_task_links() {
        let root = temp_path("success");
        let project = root.join("project");
        let store_path = root.join("automations.sqlite3");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&project).unwrap();
        let definition = definition(project);
        let occurrence = occurrence(&definition);
        let runtime = Arc::new(Runtime::fake().unwrap());
        let server = AppServer::with_feature_config(
            Arc::clone(&runtime),
            AppServerFeatureConfig {
                automations: AutomationSupervisorConfig {
                    store_path: store_path.clone(),
                    ..AutomationSupervisorConfig::default()
                },
                ..AppServerFeatureConfig::default()
            },
        );

        let store = AutomationStore::open(&store_path).unwrap();
        store.upsert_automation(&definition, None).unwrap();
        let queued = server
            .submit_automation_run(definition.clone(), occurrence)
            .await
            .unwrap();
        assert_eq!(queued.state, AutomationRunState::Queued);
        assert!(queued.task_id.is_some());
        assert!(
            server
                .tasks
                .list()
                .await
                .iter()
                .any(|task| task.spec.kind == AUTOMATION_TASK_EXECUTOR_ID)
        );

        let completed = wait_for_run_state(&store, &queued.run_id, AutomationRunState::Completed)
            .await
            .unwrap();
        assert!(completed.thread_id.is_some());
        assert!(completed.turn_id.is_some());
        assert_eq!(completed.task_id, queued.task_id);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn automation_worker_records_missing_project_as_failed() {
        let root = temp_path("missing");
        let store_path = root.join("automations.sqlite3");
        let missing_project = root.join("missing-project");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let definition = definition(missing_project);
        let occurrence = occurrence(&definition);
        let runtime = Arc::new(Runtime::fake().unwrap());
        let server = AppServer::with_feature_config(
            Arc::clone(&runtime),
            AppServerFeatureConfig {
                automations: AutomationSupervisorConfig {
                    store_path: store_path.clone(),
                    ..AutomationSupervisorConfig::default()
                },
                ..AppServerFeatureConfig::default()
            },
        );

        let store = AutomationStore::open(&store_path).unwrap();
        store.upsert_automation(&definition, None).unwrap();
        let queued = server
            .submit_automation_run(definition.clone(), occurrence)
            .await
            .unwrap();
        let failed = wait_for_run_state(&store, &queued.run_id, AutomationRunState::Failed)
            .await
            .unwrap();
        assert!(
            failed
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("project path does not exist")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn automation_worker_cancel_run_records_cancelled_state() {
        let root = temp_path("cancel");
        let project = root.join("project");
        let store_path = root.join("automations.sqlite3");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&project).unwrap();
        let definition = definition(project);
        let occurrence = occurrence(&definition);
        let runtime = Arc::new(Runtime::fake().unwrap());
        let server = AppServer::with_feature_config(
            Arc::clone(&runtime),
            AppServerFeatureConfig {
                automations: AutomationSupervisorConfig {
                    store_path: store_path.clone(),
                    ..AutomationSupervisorConfig::default()
                },
                ..AppServerFeatureConfig::default()
            },
        );
        let store = AutomationStore::open(&store_path).unwrap();
        store
            .upsert_automation(&definition, Some(occurrence.scheduled_for))
            .unwrap();
        store
            .record_occurrence(&occurrence, OffsetDateTime::now_utc())
            .unwrap();
        let run = AutomationRunSummary {
            run_id: "run-cancel".to_string(),
            automation_id: definition.id.clone(),
            occurrence_key: occurrence.occurrence_key,
            state: AutomationRunState::Queued,
            scheduled_for: occurrence.scheduled_for,
            queued_at: Some(OffsetDateTime::now_utc()),
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
        store.upsert_run(&run, OffsetDateTime::now_utc()).unwrap();

        assert!(
            server
                .cancel_automation_run("run-cancel", Some("test cancel".to_string()))
                .await
                .unwrap()
        );
        let cancelled = store.get_run(&"run-cancel".to_string()).unwrap().unwrap();
        assert_eq!(cancelled.state, AutomationRunState::Cancelled);
        assert_eq!(cancelled.error.as_deref(), Some("test cancel"));

        let _ = std::fs::remove_dir_all(&root);
    }

    async fn wait_for_run_state(
        store: &AutomationStore,
        run_id: &str,
        state: AutomationRunState,
    ) -> Option<AutomationRunSummary> {
        for _ in 0..100 {
            if let Some(run) = store.get_run(&run_id.to_string()).unwrap()
                && run.state == state
            {
                return Some(run);
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        None
    }
}
