mod executor;
mod plan;
mod script_stack;
mod state;
mod task;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use roder_api::dynamic_workflows::{
    WorkflowApproval, WorkflowCheckpointRecorded, WorkflowRun, WorkflowRunId, WorkflowRunResumed,
    WorkflowRunStatus, WorkflowScript,
};
use roder_api::events::{RoderEvent, ThreadId, TurnId};
use roder_api::subagents::{SubagentExitReason, SubagentRequest, SubagentResult};
use time::OffsetDateTime;
use tokio::sync::{Mutex, Notify, broadcast};
use tokio::task::{JoinHandle, JoinSet};

pub use self::task::WorkflowTaskExecutor;
use crate::host_api::{WorkflowAgentLaunch, WorkflowExecution};
use crate::model::{WorkflowRunInput, WorkflowRuntimeOptions};
use crate::runner::WorkflowScriptRuntime;
use crate::store::{WorkflowCachedAgentResult, WorkflowCheckpointStore};

pub use self::executor::{SubagentDispatcherWorkflowExecutor, WorkflowAgentExecutor};
use self::plan::{
    PlannedAgent, execution_context, mark_restarted_agent_completed, phases_for_execution,
    plan_agents, render_final_report,
};
use self::script_stack::run_script_on_dedicated_stack;
use self::state::{
    mark_agent_completed, mark_agent_error, mark_agent_failed, mark_agent_started,
    mark_phase_completed, mark_phase_started, mark_run_completed, mark_run_paused,
    mark_run_started, mark_run_stopped,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunRequest {
    pub run_id: WorkflowRunId,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub script: WorkflowScript,
    pub arguments: serde_json::Value,
    pub start_paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<WorkflowApproval>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunSnapshot {
    pub run: WorkflowRun,
    pub report: Option<String>,
    pub reused_agent_results: u32,
}

#[derive(Debug, Clone)]
pub struct WorkflowAgentExecutionContext {
    pub run_id: WorkflowRunId,
    pub phase_id: String,
    pub agent_id: String,
    pub thread_id: Option<ThreadId>,
    pub turn_id: Option<TurnId>,
    pub stopped: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct WorkflowAgentExecutionRequest {
    pub launch: WorkflowAgentLaunch,
    pub subagent_request: SubagentRequest,
    pub cache_key: crate::store::WorkflowAgentCacheKey,
}

#[derive(Clone)]
pub struct WorkflowRunner {
    executor: Arc<dyn WorkflowAgentExecutor>,
    script_runtime: WorkflowScriptRuntime,
    store: WorkflowCheckpointStore,
    events: broadcast::Sender<RoderEvent>,
}

impl WorkflowRunner {
    pub fn new(
        executor: Arc<dyn WorkflowAgentExecutor>,
        store: WorkflowCheckpointStore,
        options: WorkflowRuntimeOptions,
    ) -> Self {
        let (events, _) = broadcast::channel(1024);
        Self {
            executor,
            script_runtime: WorkflowScriptRuntime::new(options),
            store,
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RoderEvent> {
        self.events.subscribe()
    }

    pub async fn start(&self, request: WorkflowRunRequest) -> anyhow::Result<WorkflowRunHandle> {
        let source = request
            .script
            .body
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("workflow script body is required to run"))?;
        let checkpoints = self.store.read_checkpoints(&request.run_id)?;
        let mut input = WorkflowRunInput::new(request.run_id.clone());
        input.arguments = request.arguments.clone();
        input.checkpoints = checkpoints;
        let execution =
            run_script_on_dedicated_stack(self.script_runtime.clone(), source.to_string(), input)
                .await?;
        let planned = plan_agents(
            &request.run_id,
            &request.thread_id,
            &request.turn_id,
            &execution,
        );
        let state = Arc::new(Mutex::new(WorkflowRunSnapshot {
            run: initial_run(&request, &execution, &planned),
            report: None,
            reused_agent_results: 0,
        }));
        let control = Arc::new(WorkflowRunControl::new(request.start_paused));
        let planned = Arc::new(planned);
        let join = spawn_run(
            self.clone(),
            state.clone(),
            control.clone(),
            planned.clone(),
            execution,
        );

        Ok(WorkflowRunHandle {
            run_id: request.run_id,
            state,
            control,
            planned,
            executor: self.executor.clone(),
            store: self.store.clone(),
            events: self.events.clone(),
            join: Arc::new(Mutex::new(Some(join))),
        })
    }

    async fn run_to_completion(
        self,
        state: Arc<Mutex<WorkflowRunSnapshot>>,
        control: Arc<WorkflowRunControl>,
        planned: Arc<BTreeMap<String, PlannedAgent>>,
        execution: WorkflowExecution,
    ) -> anyhow::Result<WorkflowRunSnapshot> {
        mark_run_started(&self.events, &state).await;
        if control.paused.load(Ordering::SeqCst) {
            mark_run_paused(&self.events, &state, Some("started paused".to_string())).await;
        }
        self.persist_script_checkpoints(&state, &execution).await?;

        let phases = { state.lock().await.run.phases.clone() };
        let mut ordered_results = Vec::new();
        for phase in phases {
            if self.wait_if_paused_or_stopped(&state, &control).await? {
                return Ok(state.lock().await.clone());
            }
            mark_phase_started(&self.events, &state, &phase.phase_id).await;
            ordered_results.extend(
                self.run_phase_agents(&state, &control, &planned, &phase.phase_id)
                    .await?,
            );
            mark_phase_completed(&self.events, &state, &phase.phase_id).await;
        }

        let report = render_final_report(&execution.report, &ordered_results);
        mark_run_completed(&self.events, &state, report).await;
        Ok(state.lock().await.clone())
    }

    async fn run_phase_agents(
        &self,
        state: &Arc<Mutex<WorkflowRunSnapshot>>,
        control: &Arc<WorkflowRunControl>,
        planned: &BTreeMap<String, PlannedAgent>,
        phase_id: &str,
    ) -> anyhow::Result<Vec<(String, SubagentResult)>> {
        let max_concurrent = state.lock().await.run.limits.max_concurrent_agents.max(1) as usize;
        let mut queue = planned
            .values()
            .filter(|agent| agent.phase_id == phase_id)
            .cloned()
            .collect::<VecDeque<_>>();
        let mut active = JoinSet::new();
        let mut results = Vec::new();

        loop {
            while active.len() < max_concurrent && !queue.is_empty() {
                if self.wait_if_paused_or_stopped(state, control).await? {
                    active.abort_all();
                    return Ok(results);
                }
                let next = queue.pop_front().expect("queue is not empty");
                let run_id = { state.lock().await.run.run_id.clone() };
                if let Some(cached) = self.store.find_agent_result(&run_id, &next.cache_key)? {
                    mark_agent_completed(
                        &self.events,
                        state,
                        &next.agent_id,
                        cached.result.clone(),
                        true,
                    )
                    .await;
                    results.push((next.agent_id, cached.result));
                    continue;
                }
                mark_agent_started(&self.events, state, &next.agent_id).await;
                active.spawn(execute_planned_agent(
                    self.executor.clone(),
                    state.clone(),
                    control.stopped.clone(),
                    next,
                ));
            }

            if active.is_empty() {
                break;
            }
            let (agent_id, result) = active.join_next().await.expect("active task exists")?;
            self.record_agent_result(state, planned, &agent_id, result, &mut results)
                .await?;
            if control.stopped.load(Ordering::SeqCst) {
                active.abort_all();
                mark_run_stopped(&self.events, state, Some("stopped".to_string())).await;
                return Ok(results);
            }
        }

        Ok(results)
    }

    async fn record_agent_result(
        &self,
        state: &Arc<Mutex<WorkflowRunSnapshot>>,
        planned: &BTreeMap<String, PlannedAgent>,
        agent_id: &str,
        result: anyhow::Result<SubagentResult>,
        results: &mut Vec<(String, SubagentResult)>,
    ) -> anyhow::Result<()> {
        match result {
            Ok(result) if result.exit_reason == SubagentExitReason::Completed => {
                if let Some(planned) = planned.get(agent_id) {
                    let run_id = { state.lock().await.run.run_id.clone() };
                    self.store.append_agent_result(
                        &run_id,
                        &WorkflowCachedAgentResult {
                            key: planned.cache_key.clone(),
                            result: result.clone(),
                            completed_at: OffsetDateTime::now_utc(),
                        },
                    )?;
                }
                mark_agent_completed(&self.events, state, agent_id, result.clone(), false).await;
                results.push((agent_id.to_string(), result));
            }
            Ok(result) => {
                mark_agent_failed(
                    &self.events,
                    state,
                    agent_id,
                    result.clone(),
                    format!("subagent exited with {:?}", result.exit_reason),
                )
                .await;
                results.push((agent_id.to_string(), result));
            }
            Err(error) => {
                mark_agent_error(&self.events, state, agent_id, error.to_string()).await;
            }
        }
        Ok(())
    }

    async fn persist_script_checkpoints(
        &self,
        state: &Arc<Mutex<WorkflowRunSnapshot>>,
        execution: &WorkflowExecution,
    ) -> anyhow::Result<()> {
        for checkpoint in &execution.checkpoints {
            let run = state.lock().await.run.clone();
            self.store.append_checkpoint(&run.run_id, checkpoint)?;
            let _ = self.events.send(RoderEvent::WorkflowCheckpointRecorded(
                WorkflowCheckpointRecorded {
                    run_id: run.run_id,
                    thread_id: run.thread_id,
                    turn_id: run.turn_id,
                    phase_id: None,
                    key: checkpoint.key.clone(),
                    byte_count: checkpoint.byte_count,
                    timestamp: OffsetDateTime::now_utc(),
                },
            ));
        }
        Ok(())
    }

    async fn wait_if_paused_or_stopped(
        &self,
        state: &Arc<Mutex<WorkflowRunSnapshot>>,
        control: &Arc<WorkflowRunControl>,
    ) -> anyhow::Result<bool> {
        while control.paused.load(Ordering::SeqCst) {
            if control.stopped.load(Ordering::SeqCst) {
                mark_run_stopped(&self.events, state, Some("stopped".to_string())).await;
                return Ok(true);
            }
            control.notify.notified().await;
        }
        if control.stopped.load(Ordering::SeqCst) {
            mark_run_stopped(&self.events, state, Some("stopped".to_string())).await;
            return Ok(true);
        }
        Ok(false)
    }
}

pub struct WorkflowRunHandle {
    run_id: WorkflowRunId,
    state: Arc<Mutex<WorkflowRunSnapshot>>,
    control: Arc<WorkflowRunControl>,
    planned: Arc<BTreeMap<String, PlannedAgent>>,
    executor: Arc<dyn WorkflowAgentExecutor>,
    store: WorkflowCheckpointStore,
    events: broadcast::Sender<RoderEvent>,
    join: Arc<Mutex<Option<JoinHandle<anyhow::Result<WorkflowRunSnapshot>>>>>,
}

impl WorkflowRunHandle {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub async fn snapshot(&self) -> WorkflowRunSnapshot {
        self.state.lock().await.clone()
    }

    pub async fn wait(&self) -> anyhow::Result<WorkflowRunSnapshot> {
        let join = self.join.lock().await.take();
        if let Some(join) = join {
            return join.await?;
        }
        Ok(self.snapshot().await)
    }

    pub async fn pause(&self, reason: Option<String>) {
        self.control.paused.store(true, Ordering::SeqCst);
        mark_run_paused(&self.events, &self.state, reason).await;
    }

    pub async fn resume(&self) {
        self.control.paused.store(false, Ordering::SeqCst);
        self.control.notify.notify_waiters();
        let mut snapshot = self.state.lock().await;
        snapshot.run.status = WorkflowRunStatus::Running;
        let run = snapshot.run.clone();
        drop(snapshot);
        let _ = self
            .events
            .send(RoderEvent::WorkflowRunResumed(WorkflowRunResumed {
                run_id: run.run_id,
                thread_id: run.thread_id,
                turn_id: run.turn_id,
                status: WorkflowRunStatus::Running,
                timestamp: OffsetDateTime::now_utc(),
            }));
    }

    pub async fn stop(&self, reason: Option<String>) {
        self.control.stopped.store(true, Ordering::SeqCst);
        self.control.paused.store(false, Ordering::SeqCst);
        self.control.notify.notify_waiters();
        mark_run_stopped(&self.events, &self.state, reason).await;
    }

    pub async fn restart_agent(&self, agent_id: &str) -> anyhow::Result<bool> {
        let Some(planned) = self.planned.get(agent_id).cloned() else {
            return Ok(false);
        };
        self.store
            .invalidate_agent_results(&self.run_id, agent_id)?;
        let context = execution_context(&self.state, &planned, self.control.stopped.clone()).await;
        let result = self
            .executor
            .execute_agent(
                context,
                WorkflowAgentExecutionRequest {
                    launch: planned.launch.clone(),
                    subagent_request: planned.request.clone(),
                    cache_key: planned.cache_key.clone(),
                },
            )
            .await?;
        self.store.append_agent_result(
            &self.run_id,
            &WorkflowCachedAgentResult {
                key: planned.cache_key,
                result: result.clone(),
                completed_at: OffsetDateTime::now_utc(),
            },
        )?;
        mark_restarted_agent_completed(&self.state, agent_id, result).await;
        Ok(true)
    }
}

struct WorkflowRunControl {
    paused: AtomicBool,
    stopped: Arc<AtomicBool>,
    notify: Notify,
}

impl WorkflowRunControl {
    fn new(paused: bool) -> Self {
        Self {
            paused: AtomicBool::new(paused),
            stopped: Arc::new(AtomicBool::new(false)),
            notify: Notify::new(),
        }
    }
}

fn spawn_run(
    runner: WorkflowRunner,
    state: Arc<Mutex<WorkflowRunSnapshot>>,
    control: Arc<WorkflowRunControl>,
    planned: Arc<BTreeMap<String, PlannedAgent>>,
    execution: WorkflowExecution,
) -> JoinHandle<anyhow::Result<WorkflowRunSnapshot>> {
    tokio::spawn(async move {
        runner
            .run_to_completion(state, control, planned, execution)
            .await
    })
}

async fn execute_planned_agent(
    executor: Arc<dyn WorkflowAgentExecutor>,
    state: Arc<Mutex<WorkflowRunSnapshot>>,
    stopped: Arc<AtomicBool>,
    planned: PlannedAgent,
) -> (String, anyhow::Result<SubagentResult>) {
    let context = execution_context(&state, &planned, stopped).await;
    let request = WorkflowAgentExecutionRequest {
        launch: planned.launch,
        subagent_request: planned.request,
        cache_key: planned.cache_key,
    };
    let agent_id = context.agent_id.clone();
    (agent_id, executor.execute_agent(context, request).await)
}

fn initial_run(
    request: &WorkflowRunRequest,
    execution: &WorkflowExecution,
    planned: &BTreeMap<String, PlannedAgent>,
) -> WorkflowRun {
    let now = OffsetDateTime::now_utc();
    WorkflowRun {
        run_id: request.run_id.clone(),
        thread_id: request.thread_id.clone(),
        turn_id: request.turn_id.clone(),
        script: request.script.clone(),
        status: WorkflowRunStatus::Queued,
        limits: execution.definition.limits.clone(),
        phases: phases_for_execution(execution),
        agents: planned
            .values()
            .map(|planned| planned.agent.clone())
            .collect(),
        approval: request.approval.clone(),
        cost_estimate: None,
        summary: None,
        error: None,
        created_at: now,
        updated_at: now,
        started_at: None,
        completed_at: None,
    }
}
