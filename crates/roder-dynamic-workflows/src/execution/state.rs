use std::sync::Arc;

use roder_api::dynamic_workflows::{
    WorkflowAgentCompleted, WorkflowAgentFailed, WorkflowAgentStarted, WorkflowAgentStatus,
    WorkflowOutputRecorded, WorkflowPhaseCompleted, WorkflowPhaseStarted, WorkflowPhaseStatus,
    WorkflowRunCompleted, WorkflowRunPaused, WorkflowRunQueued, WorkflowRunStarted,
    WorkflowRunStatus, WorkflowRunStopped,
};
use roder_api::events::RoderEvent;
use roder_api::subagents::{SubagentExitReason, SubagentResult};
use time::OffsetDateTime;
use tokio::sync::{Mutex, broadcast};

use super::WorkflowRunSnapshot;
use super::plan::summary_for_run;

pub(crate) async fn mark_run_started(
    events: &broadcast::Sender<RoderEvent>,
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
) {
    let mut snapshot = state.lock().await;
    snapshot.run.status = WorkflowRunStatus::Running;
    snapshot.run.started_at = Some(OffsetDateTime::now_utc());
    snapshot.run.updated_at = OffsetDateTime::now_utc();
    let run = snapshot.run.clone();
    drop(snapshot);
    emit(
        events,
        RoderEvent::WorkflowRunQueued(WorkflowRunQueued {
            run_id: run.run_id.clone(),
            thread_id: run.thread_id.clone(),
            turn_id: run.turn_id.clone(),
            status: WorkflowRunStatus::Queued,
            timestamp: OffsetDateTime::now_utc(),
        }),
    );
    emit(
        events,
        RoderEvent::WorkflowRunStarted(WorkflowRunStarted {
            run_id: run.run_id,
            thread_id: run.thread_id,
            turn_id: run.turn_id,
            status: WorkflowRunStatus::Running,
            timestamp: OffsetDateTime::now_utc(),
        }),
    );
}

pub(crate) async fn mark_run_paused(
    events: &broadcast::Sender<RoderEvent>,
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
    reason: Option<String>,
) {
    let mut snapshot = state.lock().await;
    snapshot.run.status = WorkflowRunStatus::Paused;
    snapshot.run.updated_at = OffsetDateTime::now_utc();
    let run = snapshot.run.clone();
    drop(snapshot);
    emit(
        events,
        RoderEvent::WorkflowRunPaused(WorkflowRunPaused {
            run_id: run.run_id,
            thread_id: run.thread_id,
            turn_id: run.turn_id,
            reason,
            timestamp: OffsetDateTime::now_utc(),
        }),
    );
}

pub(crate) async fn mark_run_stopped(
    events: &broadcast::Sender<RoderEvent>,
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
    reason: Option<String>,
) {
    let mut snapshot = state.lock().await;
    if matches!(
        snapshot.run.status,
        WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Stopped
    ) {
        return;
    }
    snapshot.run.status = WorkflowRunStatus::Stopped;
    snapshot.run.completed_at = Some(OffsetDateTime::now_utc());
    snapshot.run.updated_at = OffsetDateTime::now_utc();
    let run = snapshot.run.clone();
    drop(snapshot);
    emit(
        events,
        RoderEvent::WorkflowRunStopped(WorkflowRunStopped {
            run_id: run.run_id,
            thread_id: run.thread_id,
            turn_id: run.turn_id,
            reason,
            timestamp: OffsetDateTime::now_utc(),
        }),
    );
}

pub(crate) async fn mark_run_completed(
    events: &broadcast::Sender<RoderEvent>,
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
    report: String,
) {
    let mut snapshot = state.lock().await;
    snapshot.run.status = WorkflowRunStatus::Completed;
    snapshot.run.completed_at = Some(OffsetDateTime::now_utc());
    snapshot.run.updated_at = OffsetDateTime::now_utc();
    snapshot.report = Some(report.clone());
    let summary = summary_for_run(&snapshot.run, Some(report));
    snapshot.run.summary = Some(summary.clone());
    let run = snapshot.run.clone();
    drop(snapshot);
    emit(
        events,
        RoderEvent::WorkflowOutputRecorded(WorkflowOutputRecorded {
            run_id: run.run_id.clone(),
            thread_id: run.thread_id.clone(),
            turn_id: run.turn_id.clone(),
            phase_id: None,
            output: run
                .summary
                .as_ref()
                .and_then(|summary| summary.report_preview.clone())
                .unwrap_or_default(),
            truncated: false,
            timestamp: OffsetDateTime::now_utc(),
        }),
    );
    emit(
        events,
        RoderEvent::WorkflowRunCompleted(WorkflowRunCompleted {
            run_id: run.run_id,
            thread_id: run.thread_id,
            turn_id: run.turn_id,
            summary,
            timestamp: OffsetDateTime::now_utc(),
        }),
    );
}

pub(crate) async fn mark_phase_started(
    events: &broadcast::Sender<RoderEvent>,
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
    phase_id: &str,
) {
    let mut snapshot = state.lock().await;
    let run_id = snapshot.run.run_id.clone();
    let thread_id = snapshot.run.thread_id.clone();
    let turn_id = snapshot.run.turn_id.clone();
    let Some(phase) = snapshot
        .run
        .phases
        .iter_mut()
        .find(|phase| phase.phase_id == phase_id)
    else {
        return;
    };
    phase.status = WorkflowPhaseStatus::Running;
    phase.started_at = Some(OffsetDateTime::now_utc());
    let phase = phase.clone();
    drop(snapshot);
    emit(
        events,
        RoderEvent::WorkflowPhaseStarted(WorkflowPhaseStarted {
            run_id,
            thread_id,
            turn_id,
            phase,
            timestamp: OffsetDateTime::now_utc(),
        }),
    );
}

pub(crate) async fn mark_phase_completed(
    events: &broadcast::Sender<RoderEvent>,
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
    phase_id: &str,
) {
    let mut snapshot = state.lock().await;
    let run_id = snapshot.run.run_id.clone();
    let thread_id = snapshot.run.thread_id.clone();
    let turn_id = snapshot.run.turn_id.clone();
    let Some(phase) = snapshot
        .run
        .phases
        .iter_mut()
        .find(|phase| phase.phase_id == phase_id)
    else {
        return;
    };
    phase.status = WorkflowPhaseStatus::Completed;
    phase.completed_at = Some(OffsetDateTime::now_utc());
    let phase = phase.clone();
    drop(snapshot);
    emit(
        events,
        RoderEvent::WorkflowPhaseCompleted(WorkflowPhaseCompleted {
            run_id,
            thread_id,
            turn_id,
            phase,
            timestamp: OffsetDateTime::now_utc(),
        }),
    );
}

pub(crate) async fn mark_agent_started(
    events: &broadcast::Sender<RoderEvent>,
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
    agent_id: &str,
) {
    let mut snapshot = state.lock().await;
    let run_id = snapshot.run.run_id.clone();
    let thread_id = snapshot.run.thread_id.clone();
    let turn_id = snapshot.run.turn_id.clone();
    let Some(agent) = snapshot
        .run
        .agents
        .iter_mut()
        .find(|agent| agent.agent_id == agent_id)
    else {
        return;
    };
    agent.status = WorkflowAgentStatus::Running;
    agent.started_at = Some(OffsetDateTime::now_utc());
    let agent = agent.clone();
    drop(snapshot);
    emit(
        events,
        RoderEvent::WorkflowAgentStarted(WorkflowAgentStarted {
            run_id,
            thread_id,
            turn_id,
            agent,
            timestamp: OffsetDateTime::now_utc(),
        }),
    );
}

pub(crate) async fn mark_agent_completed(
    events: &broadcast::Sender<RoderEvent>,
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
    agent_id: &str,
    result: SubagentResult,
    reused: bool,
) {
    let mut snapshot = state.lock().await;
    let run_id = snapshot.run.run_id.clone();
    let thread_id = snapshot.run.thread_id.clone();
    let turn_id = snapshot.run.turn_id.clone();
    if reused {
        snapshot.reused_agent_results = snapshot.reused_agent_results.saturating_add(1);
    }
    let Some(agent) = snapshot
        .run
        .agents
        .iter_mut()
        .find(|agent| agent.agent_id == agent_id)
    else {
        return;
    };
    agent.status = WorkflowAgentStatus::Completed;
    agent.thread_id = Some(result.thread_id);
    agent.turn_id = Some(result.turn_id);
    agent.usage = result.usage;
    agent.exit_reason = Some(result.exit_reason);
    agent.completed_at = Some(OffsetDateTime::now_utc());
    let phase_id = agent.phase_id.clone();
    let agent = agent.clone();
    if let Some(phase) = snapshot
        .run
        .phases
        .iter_mut()
        .find(|phase| phase.phase_id == phase_id)
    {
        phase.completed_agents = phase.completed_agents.saturating_add(1);
    }
    drop(snapshot);
    emit(
        events,
        RoderEvent::WorkflowAgentCompleted(WorkflowAgentCompleted {
            run_id,
            thread_id,
            turn_id,
            agent,
            timestamp: OffsetDateTime::now_utc(),
        }),
    );
}

pub(crate) async fn mark_agent_failed(
    events: &broadcast::Sender<RoderEvent>,
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
    agent_id: &str,
    result: SubagentResult,
    error: String,
) {
    let mut snapshot = state.lock().await;
    let run_id = snapshot.run.run_id.clone();
    let thread_id = snapshot.run.thread_id.clone();
    let turn_id = snapshot.run.turn_id.clone();
    let Some(agent) = snapshot
        .run
        .agents
        .iter_mut()
        .find(|agent| agent.agent_id == agent_id)
    else {
        return;
    };
    agent.status = match result.exit_reason {
        SubagentExitReason::Timeout => WorkflowAgentStatus::Timeout,
        SubagentExitReason::Cancelled => WorkflowAgentStatus::Cancelled,
        _ => WorkflowAgentStatus::Failed,
    };
    agent.thread_id = Some(result.thread_id);
    agent.turn_id = Some(result.turn_id);
    agent.usage = result.usage;
    agent.exit_reason = Some(result.exit_reason);
    agent.error = Some(error.clone());
    agent.completed_at = Some(OffsetDateTime::now_utc());
    let phase_id = agent.phase_id.clone();
    let agent = agent.clone();
    if let Some(phase) = snapshot
        .run
        .phases
        .iter_mut()
        .find(|phase| phase.phase_id == phase_id)
    {
        phase.failed_agents = phase.failed_agents.saturating_add(1);
    }
    drop(snapshot);
    emit(
        events,
        RoderEvent::WorkflowAgentFailed(WorkflowAgentFailed {
            run_id,
            thread_id,
            turn_id,
            agent,
            error,
            timestamp: OffsetDateTime::now_utc(),
        }),
    );
}

pub(crate) async fn mark_agent_error(
    events: &broadcast::Sender<RoderEvent>,
    state: &Arc<Mutex<WorkflowRunSnapshot>>,
    agent_id: &str,
    error: String,
) {
    let result = SubagentResult {
        thread_id: "unknown".to_string(),
        turn_id: agent_id.to_string(),
        agent_type: "unknown".to_string(),
        model: None,
        final_message: error.clone(),
        usage: None,
        exit_reason: SubagentExitReason::Failed,
        transcript: None,
        metadata: serde_json::json!({}),
    };
    mark_agent_failed(events, state, agent_id, result, error).await;
}

fn emit(events: &broadcast::Sender<RoderEvent>, event: RoderEvent) {
    let _ = events.send(event);
}
