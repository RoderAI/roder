use std::{collections::HashMap, sync::Arc};

use roder_api::{
    dynamic_workflows::{
        WorkflowApproval, WorkflowApprovalDecision, WorkflowApprovalRequested, WorkflowRun,
        WorkflowRunApproved, WorkflowRunDenied, WorkflowRunDrafted, WorkflowRunId,
        WorkflowRunQueued, WorkflowRunStatus, WorkflowScriptSourceKind,
    },
    events::RoderEvent,
};
use roder_commands::{WorkflowCommandSaveRequest, save_workflow_command};
use roder_core::Runtime;
use roder_dynamic_workflows::{
    WorkflowCheckpointStore, WorkflowRunHandle, WorkflowRunRequest, WorkflowRunner,
    WorkflowRuntimeOptions,
};
use roder_protocol::*;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use uuid::Uuid;

use super::scripts;
use super::support::{
    AppWorkflowExecutor, approval_id, cost_estimate_for_source, drafted_run, is_terminal,
    prompt_workflow_source, script_from_source, summary_for_run,
};

pub(crate) struct AppWorkflowService {
    runtime: Arc<Runtime>,
    runner: WorkflowRunner,
    runs: RwLock<HashMap<WorkflowRunId, StoredWorkflowRun>>,
}

enum StoredWorkflowRun {
    Draft {
        run: WorkflowRun,
        arguments: serde_json::Value,
    },
    Active(Arc<WorkflowRunHandle>),
    Terminal(WorkflowRun),
}

impl AppWorkflowService {
    pub(crate) fn new(runtime: Arc<Runtime>) -> Self {
        let executor = Arc::new(AppWorkflowExecutor {
            dispatcher: runtime.registry().subagent_dispatchers.first().cloned(),
        });
        let runner = WorkflowRunner::new(
            executor,
            WorkflowCheckpointStore::new(roder_config::config_dir()),
            workflow_runtime_options_from_config(),
        );
        if tokio::runtime::Handle::try_current().is_ok() {
            let mut events = runner.subscribe();
            let event_runtime = runtime.clone();
            tokio::spawn(async move {
                while let Ok(event) = events.recv().await {
                    event_runtime.emit(event).await;
                }
            });
        }
        Self {
            runtime,
            runner,
            runs: RwLock::new(HashMap::new()),
        }
    }

    pub(crate) async fn plan(
        &self,
        params: WorkflowsPlanParams,
    ) -> anyhow::Result<WorkflowsPlanResult> {
        let source = params
            .script
            .unwrap_or_else(|| prompt_workflow_source(&params.prompt));
        let script = script_from_source(
            &source,
            WorkflowScriptSourceKind::Generated,
            params.workspace.clone(),
            None,
        )?;
        let run = drafted_run(
            format!("workflow-{}", Uuid::new_v4()),
            params.thread_id,
            params.turn_id,
            script,
            cost_estimate_for_source(&source),
        );
        let arguments = params.arguments;
        self.runs.write().await.insert(
            run.run_id.clone(),
            StoredWorkflowRun::Draft {
                run: run.clone(),
                arguments,
            },
        );
        self.runtime
            .emit(RoderEvent::WorkflowRunDrafted(WorkflowRunDrafted {
                run_id: run.run_id.clone(),
                thread_id: run.thread_id.clone(),
                turn_id: run.turn_id.clone(),
                run: run.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        self.runtime
            .emit(RoderEvent::WorkflowApprovalRequested(
                WorkflowApprovalRequested {
                    run_id: run.run_id.clone(),
                    thread_id: run.thread_id.clone(),
                    turn_id: run.turn_id.clone(),
                    approval_id: approval_id(&run.run_id),
                    run: run.clone(),
                    timestamp: OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(WorkflowsPlanResult {
            run,
            approval_required: true,
        })
    }

    pub(crate) async fn approve(
        &self,
        params: WorkflowsApproveParams,
    ) -> anyhow::Result<WorkflowsApproveResult> {
        let (run, arguments) = match self.runs.write().await.remove(&params.run_id) {
            Some(StoredWorkflowRun::Draft { run, arguments }) => (run, arguments),
            Some(other) => {
                let run = run_from_stored(&other).await;
                self.runs.write().await.insert(run.run_id.clone(), other);
                anyhow::bail!("workflow {:?} is not awaiting approval", params.run_id);
            }
            None => anyhow::bail!("unknown workflow {:?}", params.run_id),
        };
        let approval = WorkflowApproval {
            approval_id: approval_id(&run.run_id),
            run_id: run.run_id.clone(),
            script_hash: run.script.hash.clone(),
            workspace: run.script.source.path.clone(),
            decision: params.decision,
            approved_capabilities: vec!["childAgents".to_string()],
            reason: params.reason,
            decided_at: OffsetDateTime::now_utc(),
        };
        if approval.decision == WorkflowApprovalDecision::Deny {
            let mut denied = run.clone();
            denied.status = WorkflowRunStatus::Failed;
            denied.approval = Some(approval.clone());
            denied.error = Some("workflow approval denied".to_string());
            denied.completed_at = Some(OffsetDateTime::now_utc());
            self.runs.write().await.insert(
                denied.run_id.clone(),
                StoredWorkflowRun::Terminal(denied.clone()),
            );
            self.runtime
                .emit(RoderEvent::WorkflowRunDenied(WorkflowRunDenied {
                    run_id: denied.run_id.clone(),
                    thread_id: denied.thread_id.clone(),
                    turn_id: denied.turn_id.clone(),
                    approval: approval.clone(),
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
            return Ok(WorkflowsApproveResult {
                run: denied,
                approval,
            });
        }

        self.runtime
            .emit(RoderEvent::WorkflowRunApproved(WorkflowRunApproved {
                run_id: run.run_id.clone(),
                thread_id: run.thread_id.clone(),
                turn_id: run.turn_id.clone(),
                approval: approval.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        self.runtime
            .emit(RoderEvent::WorkflowRunQueued(WorkflowRunQueued {
                run_id: run.run_id.clone(),
                thread_id: run.thread_id.clone(),
                turn_id: run.turn_id.clone(),
                status: WorkflowRunStatus::Queued,
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        let handle = Arc::new(
            self.runner
                .start(WorkflowRunRequest {
                    run_id: run.run_id.clone(),
                    thread_id: run.thread_id.clone(),
                    turn_id: run.turn_id.clone(),
                    script: run.script.clone(),
                    arguments,
                    start_paused: false,
                    approval: Some(approval.clone()),
                })
                .await?,
        );
        let snapshot = handle.snapshot().await.run;
        self.runs
            .write()
            .await
            .insert(run.run_id.clone(), StoredWorkflowRun::Active(handle));
        Ok(WorkflowsApproveResult {
            run: snapshot,
            approval,
        })
    }

    pub(crate) async fn list(
        &self,
        params: WorkflowsListParams,
    ) -> anyhow::Result<WorkflowsListResult> {
        let mut runs = Vec::new();
        for stored in self.runs.read().await.values() {
            let run = run_from_stored(stored).await;
            if let Some(thread_id) = &params.thread_id
                && run.thread_id.as_ref() != Some(thread_id)
            {
                continue;
            }
            if !params.include_terminal && is_terminal(run.status) {
                continue;
            }
            runs.push(summary_for_run(&run));
        }
        runs.sort_by(|a, b| a.run_id.cmp(&b.run_id));
        Ok(WorkflowsListResult { runs })
    }

    pub(crate) async fn get(
        &self,
        params: WorkflowsGetParams,
    ) -> anyhow::Result<WorkflowsGetResult> {
        let mut run = self.run(&params.run_id).await?;
        if !params.include_script_body {
            run.script.body = None;
        }
        if !params.include_agents {
            run.agents.clear();
        }
        Ok(WorkflowsGetResult { run })
    }

    pub(crate) async fn pause(
        &self,
        params: WorkflowsPauseParams,
    ) -> anyhow::Result<WorkflowsPauseResult> {
        let handle = self.active(&params.run_id).await?;
        handle.pause(params.reason).await;
        Ok(WorkflowsPauseResult {
            run: handle.snapshot().await.run,
        })
    }

    pub(crate) async fn resume(
        &self,
        params: WorkflowsResumeParams,
    ) -> anyhow::Result<WorkflowsResumeResult> {
        let handle = self.active(&params.run_id).await?;
        handle.resume().await;
        Ok(WorkflowsResumeResult {
            run: handle.snapshot().await.run,
        })
    }

    pub(crate) async fn stop(
        &self,
        params: WorkflowsStopParams,
    ) -> anyhow::Result<WorkflowsStopResult> {
        let handle = self.active(&params.run_id).await?;
        handle.stop(params.reason).await;
        Ok(WorkflowsStopResult {
            run: handle.snapshot().await.run,
        })
    }

    pub(crate) async fn restart_agent(
        &self,
        params: WorkflowsRestartAgentParams,
    ) -> anyhow::Result<WorkflowsRestartAgentResult> {
        let handle = self.active(&params.run_id).await?;
        if !handle.restart_agent(&params.agent_id).await? {
            anyhow::bail!("unknown workflow agent {:?}", params.agent_id);
        }
        let run = handle.snapshot().await.run;
        let agent = run
            .agents
            .iter()
            .find(|agent| agent.agent_id == params.agent_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown workflow agent {:?}", params.agent_id))?;
        Ok(WorkflowsRestartAgentResult { run, agent })
    }

    pub(crate) async fn save(
        &self,
        params: WorkflowsSaveParams,
    ) -> anyhow::Result<WorkflowsSaveResult> {
        let run = self.run(&params.run_id).await?;
        if params.name != run.script.name {
            anyhow::bail!(
                "workflow save name {:?} must match script metadata name {:?}",
                params.name,
                run.script.name
            );
        }
        let config = roder_config::load_config()?;
        let workspace = self.runtime.workspace();
        let workflow_dirs = roder_config::dynamic_workflows::resolve_workflow_directories(
            config.dynamic_workflows.as_ref(),
            Some(&workspace),
        );
        let target_dir = match params.scope {
            WorkflowsSaveScope::User => workflow_dirs.user,
            WorkflowsSaveScope::Workspace => workflow_dirs.workspace,
        };
        let path = save_workflow_command(WorkflowCommandSaveRequest {
            target_dir,
            script: run.script.clone(),
            overwrite: params.overwrite,
        })?;
        let mut script = run.script;
        script.source.kind = match params.scope {
            WorkflowsSaveScope::User => WorkflowScriptSourceKind::User,
            WorkflowsSaveScope::Workspace => WorkflowScriptSourceKind::Workspace,
        };
        script.source.path = Some(path.display().to_string());
        script.updated_at = OffsetDateTime::now_utc();
        Ok(WorkflowsSaveResult { script })
    }

    pub(crate) fn scripts_list(
        &self,
        params: WorkflowsScriptsListParams,
    ) -> anyhow::Result<WorkflowsScriptsListResult> {
        scripts::list_scripts(&self.runtime, params)
    }

    pub(crate) fn scripts_read(
        &self,
        params: WorkflowsScriptsReadParams,
    ) -> anyhow::Result<WorkflowsScriptsReadResult> {
        scripts::read_script(&self.runtime, params)
    }

    pub(crate) fn scripts_delete(
        &self,
        params: WorkflowsScriptsDeleteParams,
    ) -> anyhow::Result<WorkflowsScriptsDeleteResult> {
        scripts::delete_script(&self.runtime, params)
    }

    async fn run(&self, run_id: &str) -> anyhow::Result<WorkflowRun> {
        let runs = self.runs.read().await;
        let Some(stored) = runs.get(run_id) else {
            anyhow::bail!("unknown workflow {run_id:?}");
        };
        Ok(run_from_stored(stored).await)
    }

    async fn active(&self, run_id: &str) -> anyhow::Result<Arc<WorkflowRunHandle>> {
        let runs = self.runs.read().await;
        match runs.get(run_id) {
            Some(StoredWorkflowRun::Active(handle)) => Ok(handle.clone()),
            Some(_) => anyhow::bail!("workflow {run_id:?} is not running"),
            None => anyhow::bail!("unknown workflow {run_id:?}"),
        }
    }
}

fn workflow_runtime_options_from_config() -> WorkflowRuntimeOptions {
    let mut options = WorkflowRuntimeOptions::default();
    if let Ok(config) = roder_config::load_config()
        && let Some(dynamic_workflows) = config.dynamic_workflows
    {
        options.limits = dynamic_workflows.limits();
        options.max_report_bytes = dynamic_workflows.max_report_bytes;
    }
    options
}

async fn run_from_stored(stored: &StoredWorkflowRun) -> WorkflowRun {
    match stored {
        StoredWorkflowRun::Draft { run, .. } | StoredWorkflowRun::Terminal(run) => run.clone(),
        StoredWorkflowRun::Active(handle) => handle.snapshot().await.run,
    }
}
