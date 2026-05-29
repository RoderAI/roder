use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use roder_api::{
    dynamic_workflows::{
        WorkflowAgentRun, WorkflowAgentStatus, WorkflowCostEstimate, WorkflowPhase,
        WorkflowPhaseStatus, WorkflowRun, WorkflowRunLimits, WorkflowRunStatus, WorkflowRunSummary,
        WorkflowScript, WorkflowScriptSource, WorkflowScriptSourceKind,
    },
    inference::TokenUsage,
};
use time::OffsetDateTime;

use super::{
    detail::detail_lines,
    render::approval_lines,
    state::{WorkflowApprovalView, WorkflowUiAction, WorkflowUiState},
};
use crate::app::Theme;

#[test]
fn trigger_detects_workflow_word_and_can_be_ignored_once() {
    let mut state = WorkflowUiState::default();
    assert!(state.trigger_active("please use a workflow for this audit"));
    state.ignore_trigger("please use a workflow for this audit");
    assert!(!state.trigger_active("please use a workflow for this audit"));
    state.reset_ignored_trigger_if_changed("different workflow request");
    assert!(state.trigger_active("different workflow request"));
    assert!(!state.trigger_active("/workflows"));
}

#[test]
fn approval_lines_include_actions_and_script_preview() {
    let mut run = workflow_run("run-approval", WorkflowRunStatus::AwaitingApproval);
    run.script.body = Some("workflow.define({}, async () => 'ok');".to_string());
    let approval = WorkflowApprovalView {
        approval_id: "approval-run".to_string(),
        run,
        prompt: "run workflow".to_string(),
        show_script: true,
    };

    let rendered = render_text(approval_lines(&approval, Theme::for_terminal()));

    assert!(rendered.contains("run once"));
    assert!(rendered.contains("always"));
    assert!(rendered.contains("Script preview"));
    assert!(rendered.contains("approval-run"));
}

#[test]
fn event_updates_active_run_progress_state() {
    let mut state = WorkflowUiState::default();
    let run = workflow_run("run-progress", WorkflowRunStatus::AwaitingApproval);
    state.record_run(run);

    state.apply_event(&roder_api::events::RoderEvent::WorkflowRunStarted(
        roder_api::dynamic_workflows::WorkflowRunStarted {
            run_id: "run-progress".to_string(),
            thread_id: None,
            turn_id: None,
            status: WorkflowRunStatus::Running,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        },
    ));

    assert_eq!(state.progress_height(), 1);
    assert_eq!(
        state.cached_run("run-progress").unwrap().status,
        WorkflowRunStatus::Running
    );
}

#[test]
fn detail_selection_exposes_agent_trace_and_restart_action() {
    let mut state = WorkflowUiState::default();
    let mut run = workflow_run("run-detail", WorkflowRunStatus::Running);
    run.agents.push(workflow_agent("agent-1"));
    state.show_detail(run.clone());
    state.move_selection(1);

    assert_eq!(
        state.selected_agent_id(),
        Some(("run-detail".to_string(), "agent-1".to_string()))
    );
    assert_eq!(
        state.key_action(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        Some(WorkflowUiAction::RestartSelectedAgent)
    );

    let rendered = render_text(detail_lines(&run, 1, Theme::for_terminal()));
    assert!(rendered.contains("trace thread-a/turn-a"));
    assert!(rendered.contains("usage 11 prompt + 7 completion = 18 total"));
}

fn render_text(text: ratatui::text::Text<'static>) -> String {
    text.lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn workflow_agent(agent_id: &str) -> WorkflowAgentRun {
    WorkflowAgentRun {
        agent_id: agent_id.to_string(),
        phase_id: "phase-1".to_string(),
        description: "inspect package for workflow regressions".to_string(),
        status: WorkflowAgentStatus::Completed,
        lane: None,
        model: Some("fake-model".to_string()),
        thread_id: Some("thread-a".to_string()),
        turn_id: Some("turn-a".to_string()),
        usage: Some(TokenUsage::new(11, 7, 18)),
        exit_reason: None,
        error: None,
        started_at: Some(OffsetDateTime::UNIX_EPOCH),
        completed_at: Some(OffsetDateTime::UNIX_EPOCH),
    }
}

fn workflow_run(run_id: &str, status: WorkflowRunStatus) -> WorkflowRun {
    WorkflowRun {
        run_id: run_id.to_string(),
        thread_id: None,
        turn_id: None,
        script: WorkflowScript {
            script_id: "script-1".to_string(),
            name: "audit-workflow".to_string(),
            description: Some("Audit workflow".to_string()),
            source: WorkflowScriptSource {
                kind: WorkflowScriptSourceKind::Generated,
                path: None,
                command_name: None,
                extension_id: None,
            },
            hash: "hash".to_string(),
            host_api_version: 1,
            arguments_schema: serde_json::json!({ "type": "object" }),
            body: None,
            limits: WorkflowRunLimits::default(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        },
        status,
        limits: WorkflowRunLimits::default(),
        phases: vec![WorkflowPhase {
            phase_id: "phase-1".to_string(),
            name: "audit".to_string(),
            status: WorkflowPhaseStatus::Queued,
            description: None,
            queued_agents: 1,
            completed_agents: 1,
            failed_agents: 0,
            started_at: None,
            completed_at: None,
        }],
        agents: Vec::new(),
        approval: None,
        cost_estimate: Some(WorkflowCostEstimate {
            min_child_agents: 1,
            max_child_agents: 3,
            estimated_prompt_tokens: Some(1200),
            estimated_completion_tokens: None,
            warning: None,
        }),
        summary: Some(WorkflowRunSummary {
            run_id: run_id.to_string(),
            status,
            title: "audit-workflow".to_string(),
            phase_count: 1,
            completed_phase_count: 0,
            agent_count: 1,
            completed_agent_count: 0,
            failed_agent_count: 0,
            concurrency_peak: 1,
            usage: Some(TokenUsage::new(11, 7, 18)),
            elapsed_ms: Some(1200),
            report_preview: Some("report preview".to_string()),
        }),
        error: None,
        created_at: OffsetDateTime::UNIX_EPOCH,
        updated_at: OffsetDateTime::UNIX_EPOCH,
        started_at: None,
        completed_at: None,
    }
}
