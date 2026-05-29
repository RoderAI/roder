use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use roder_api::dynamic_workflows::{
    WorkflowAgentRun, WorkflowAgentRunId, WorkflowAgentStatus, WorkflowPhase, WorkflowPhaseStatus,
    WorkflowRun, WorkflowRunSummary,
};
use roder_api::events::{ThreadId, TurnId};
use roder_api::inference::TokenUsage;
use roder_api::subagents::{SubagentLane, SubagentRequest, SubagentResult};
use time::OffsetDateTime;
use tokio::sync::Mutex;

use crate::approval::workflow_script_hash;
use crate::host_api::{WorkflowAgentLaunch, WorkflowExecution};
use crate::store::WorkflowAgentCacheKey;

use super::{WorkflowAgentExecutionContext, WorkflowRunSnapshot};

#[derive(Debug, Clone)]
pub(crate) struct PlannedAgent {
    pub(crate) phase_id: String,
    pub(crate) agent_id: WorkflowAgentRunId,
    pub(crate) launch: WorkflowAgentLaunch,
    pub(crate) request: SubagentRequest,
    pub(crate) cache_key: WorkflowAgentCacheKey,
    pub(crate) agent: WorkflowAgentRun,
}

pub(crate) fn plan_agents(
    run_id: &str,
    thread_id: &Option<ThreadId>,
    turn_id: &Option<TurnId>,
    execution: &WorkflowExecution,
) -> BTreeMap<String, PlannedAgent> {
    let phases = phases_for_execution(execution);
    let phase_by_name = phases
        .iter()
        .map(|phase| (phase.name.clone(), phase.phase_id.clone()))
        .collect::<HashMap<_, _>>();
    let default_phase_id = phases
        .first()
        .map(|phase| phase.phase_id.clone())
        .unwrap_or_else(|| "phase-1".to_string());

    execution
        .agent_launches
        .iter()
        .enumerate()
        .map(|(idx, launch)| {
            let agent_id = format!("agent-{}", idx + 1);
            let lane = parse_lane(&launch.lane);
            let tool_scope = lane
                .preset()
                .allowed_tools
                .iter()
                .map(|tool| (*tool).to_string())
                .collect::<Vec<_>>();
            let phase_id = launch
                .phase
                .as_ref()
                .and_then(|name| phase_by_name.get(name))
                .cloned()
                .unwrap_or_else(|| default_phase_id.clone());
            let request = SubagentRequest {
                description: non_empty_or(&launch.description, &launch.role),
                prompt: launch.prompt.clone(),
                subagent_type: None,
                model: launch.model.clone(),
                tools: None,
                lane: Some(lane),
                max_concurrent: Some(execution.definition.limits.max_concurrent_agents as usize),
                allowed_tools: Some(tool_scope.clone()),
                parent_deadline_seconds: Some(
                    execution.definition.limits.default_run_timeout_seconds,
                ),
                inputs: Some(launch.input.clone()),
                timeout_seconds: Some(launch.timeout_seconds),
            };
            let cache_key = WorkflowAgentCacheKey {
                run_id: run_id.to_string(),
                phase_id: phase_id.clone(),
                agent_id: agent_id.clone(),
                prompt_hash: workflow_script_hash(&launch.prompt),
                model: launch.model.clone(),
                tool_scope,
            };
            let agent = WorkflowAgentRun {
                agent_id: agent_id.clone(),
                phase_id: phase_id.clone(),
                description: non_empty_or(&launch.description, &launch.prompt),
                status: WorkflowAgentStatus::Queued,
                lane: Some(lane),
                model: launch.model.clone(),
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                usage: None,
                exit_reason: None,
                error: None,
                started_at: None,
                completed_at: None,
            };
            (
                agent_id.clone(),
                PlannedAgent {
                    phase_id,
                    agent_id,
                    launch: launch.clone(),
                    request,
                    cache_key,
                    agent,
                },
            )
        })
        .collect()
}

pub(crate) fn phases_for_execution(execution: &WorkflowExecution) -> Vec<WorkflowPhase> {
    let names = if !execution.phases.is_empty() {
        execution.phases.clone()
    } else if !execution.definition.phases.is_empty() {
        execution.definition.phases.clone()
    } else {
        vec![execution.definition.name.clone()]
    };
    names
        .into_iter()
        .enumerate()
        .map(|(idx, name)| WorkflowPhase {
            phase_id: format!("phase-{}", idx + 1),
            name,
            status: WorkflowPhaseStatus::Queued,
            description: None,
            queued_agents: 0,
            completed_agents: 0,
            failed_agents: 0,
            started_at: None,
            completed_at: None,
        })
        .collect()
}

pub(crate) async fn execution_context(
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
    planned: &PlannedAgent,
    stopped: Arc<AtomicBool>,
) -> WorkflowAgentExecutionContext {
    let run = state.lock().await.run.clone();
    WorkflowAgentExecutionContext {
        run_id: run.run_id,
        phase_id: planned.phase_id.clone(),
        agent_id: planned.agent_id.clone(),
        thread_id: run.thread_id,
        turn_id: run.turn_id,
        stopped,
    }
}

pub(crate) async fn mark_restarted_agent_completed(
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
    agent_id: &str,
    result: SubagentResult,
) {
    let mut snapshot = state.lock().await;
    if let Some(agent) = snapshot
        .run
        .agents
        .iter_mut()
        .find(|agent| agent.agent_id == agent_id)
    {
        agent.status = WorkflowAgentStatus::Completed;
        agent.thread_id = Some(result.thread_id);
        agent.turn_id = Some(result.turn_id);
        agent.usage = result.usage;
        agent.exit_reason = Some(result.exit_reason);
        agent.completed_at = Some(OffsetDateTime::now_utc());
    }
}

pub(crate) fn summary_for_run(run: &WorkflowRun, report: Option<String>) -> WorkflowRunSummary {
    let mut usage = TokenUsage::default();
    for agent in &run.agents {
        if let Some(agent_usage) = &agent.usage {
            usage.add_assign(agent_usage);
        }
    }
    WorkflowRunSummary {
        run_id: run.run_id.clone(),
        status: run.status,
        title: run.script.name.clone(),
        phase_count: run.phases.len() as u32,
        completed_phase_count: run
            .phases
            .iter()
            .filter(|phase| phase.status == WorkflowPhaseStatus::Completed)
            .count() as u32,
        agent_count: run.agents.len() as u32,
        completed_agent_count: run
            .agents
            .iter()
            .filter(|agent| agent.status == WorkflowAgentStatus::Completed)
            .count() as u32,
        failed_agent_count: run
            .agents
            .iter()
            .filter(|agent| {
                matches!(
                    agent.status,
                    WorkflowAgentStatus::Failed
                        | WorkflowAgentStatus::Timeout
                        | WorkflowAgentStatus::Cancelled
                )
            })
            .count() as u32,
        concurrency_peak: run.limits.max_concurrent_agents,
        usage: (!usage.is_empty()).then_some(usage),
        elapsed_ms: elapsed_ms(run),
        report_preview: report.map(|report| report.chars().take(512).collect()),
    }
}

pub(crate) fn render_final_report(
    script_report: &str,
    results: &[(String, SubagentResult)],
) -> String {
    if results.is_empty() {
        return script_report.to_string();
    }
    let mut report = script_report.to_string();
    report.push_str("\n\n## Child Agent Results\n");
    for (agent_id, result) in results {
        report.push_str(&format!(
            "\n- {agent_id} ({:?}): {}",
            result.exit_reason,
            result.final_message.replace('\n', " ")
        ));
    }
    report
}

fn elapsed_ms(run: &WorkflowRun) -> Option<u64> {
    let started = run.started_at?;
    let completed = run.completed_at.unwrap_or_else(OffsetDateTime::now_utc);
    (completed - started).whole_milliseconds().try_into().ok()
}

fn parse_lane(value: &str) -> SubagentLane {
    match value.trim().to_ascii_lowercase().as_str() {
        "editor" => SubagentLane::Editor,
        "reviewer" => SubagentLane::Reviewer,
        "runner" => SubagentLane::Runner,
        _ => SubagentLane::Scout,
    }
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}
