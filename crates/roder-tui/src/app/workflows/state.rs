use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{Frame, layout::Rect, widgets::Paragraph};
use roder_api::{
    dynamic_workflows::{
        WorkflowAgentRun, WorkflowApprovalDecision, WorkflowPhase, WorkflowRun, WorkflowRunId,
        WorkflowRunStatus, WorkflowRunSummary,
    },
    events::RoderEvent,
};

use super::super::{Theme, short_id};
use super::render;

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkflowUiState {
    pub(super) runs: BTreeMap<WorkflowRunId, WorkflowRun>,
    pub(super) summaries: BTreeMap<WorkflowRunId, WorkflowRunSummary>,
    pub(super) approval: Option<WorkflowApprovalView>,
    pub(super) panel: WorkflowPanel,
    pub(super) selected: usize,
    pub(super) detail_selected: usize,
    ignored_prompt: Option<String>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(super) enum WorkflowPanel {
    #[default]
    Hidden,
    List,
    Detail(WorkflowRunId),
}

#[derive(Debug, Clone)]
pub(super) struct WorkflowApprovalView {
    pub(super) approval_id: String,
    pub(super) run: WorkflowRun,
    pub(super) prompt: String,
    pub(super) show_script: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) enum WorkflowUiAction {
    Approve(WorkflowApprovalDecision),
    Deny,
    EditPrompt,
    ToggleScript,
    Close,
    Back,
    RefreshList,
    OpenSelected,
    MoveSelection(isize),
    PauseSelected,
    ResumeSelected,
    StopSelected,
    SaveSelected,
    RestartSelectedAgent,
}

impl WorkflowUiState {
    pub(crate) fn overlay_visible(&self) -> bool {
        self.approval.is_some() || !matches!(self.panel, WorkflowPanel::Hidden)
    }

    pub(crate) fn progress_height(&self, width: u16, frame_height: u16) -> u16 {
        super::progress::progress_height(self, width, frame_height)
    }

    pub(crate) fn trigger_height(&self, composer_text: &str) -> u16 {
        u16::from(self.trigger_active(composer_text))
    }

    pub(super) fn trigger_active(&self, composer_text: &str) -> bool {
        let trimmed = composer_text.trim();
        !trimmed.is_empty()
            && !trimmed.starts_with('/')
            && self.ignored_prompt.as_deref() != Some(trimmed)
            && contains_workflow_trigger(trimmed)
    }

    pub(super) fn ignore_trigger(&mut self, composer_text: &str) {
        self.ignored_prompt = Some(composer_text.trim().to_string());
    }

    pub(super) fn reset_ignored_trigger_if_changed(&mut self, composer_text: &str) {
        if self.ignored_prompt.as_deref() != Some(composer_text.trim()) {
            self.ignored_prompt = None;
        }
    }

    pub(super) fn start_approval(&mut self, approval_id: String, run: WorkflowRun, prompt: String) {
        self.record_run(run.clone());
        self.approval = Some(WorkflowApprovalView {
            approval_id,
            run,
            prompt,
            show_script: false,
        });
    }

    pub(super) fn clear_approval(&mut self) {
        self.approval = None;
    }

    pub(super) fn approval_run_id(&self) -> Option<WorkflowRunId> {
        self.approval
            .as_ref()
            .map(|approval| approval.run.run_id.clone())
    }

    pub(super) fn approval_prompt(&self) -> Option<String> {
        self.approval
            .as_ref()
            .map(|approval| approval.prompt.clone())
    }

    pub(super) fn cached_run(&self, run_id: &str) -> Option<WorkflowRun> {
        self.runs.get(run_id).cloned()
    }

    pub(super) fn set_list(&mut self, runs: Vec<WorkflowRunSummary>) {
        self.summaries = runs
            .iter()
            .cloned()
            .map(|summary| (summary.run_id.clone(), summary))
            .collect();
        self.selected = self.selected.min(runs.len().saturating_sub(1));
        self.panel = WorkflowPanel::List;
    }

    pub(super) fn show_detail(&mut self, run: WorkflowRun) {
        let run_id = run.run_id.clone();
        self.record_run(run);
        self.detail_selected = self
            .detail_selected
            .min(self.detail_item_count(&run_id).saturating_sub(1));
        self.panel = WorkflowPanel::Detail(run_id);
    }

    pub(super) fn record_run(&mut self, run: WorkflowRun) {
        self.summaries
            .insert(run.run_id.clone(), summary_from_run(&run));
        self.runs.insert(run.run_id.clone(), run);
    }

    pub(crate) fn apply_event(&mut self, event: &RoderEvent) -> Option<String> {
        match event {
            RoderEvent::WorkflowRunDrafted(event) => {
                self.record_run(event.run.clone());
                Some(format!("workflow drafted: {}", short_id(&event.run_id)))
            }
            RoderEvent::WorkflowApprovalRequested(event) => {
                self.record_run(event.run.clone());
                if self.approval_run_id().as_deref() != Some(&event.run_id) {
                    self.approval = Some(WorkflowApprovalView {
                        approval_id: event.approval_id.clone(),
                        run: event.run.clone(),
                        prompt: event.run.script.description.clone().unwrap_or_default(),
                        show_script: false,
                    });
                }
                Some(format!(
                    "workflow approval requested: {}",
                    short_id(&event.run_id)
                ))
            }
            RoderEvent::WorkflowRunApproved(event) => {
                self.update_status(&event.run_id, WorkflowRunStatus::Queued);
                if self.approval_run_id().as_deref() == Some(&event.run_id) {
                    self.clear_approval();
                }
                Some(format!("workflow approved: {}", short_id(&event.run_id)))
            }
            RoderEvent::WorkflowRunDenied(event) => {
                self.update_status(&event.run_id, WorkflowRunStatus::Failed);
                if self.approval_run_id().as_deref() == Some(&event.run_id) {
                    self.clear_approval();
                }
                Some(format!("workflow denied: {}", short_id(&event.run_id)))
            }
            RoderEvent::WorkflowRunQueued(event) => {
                self.update_status(&event.run_id, event.status);
                Some(format!("workflow queued: {}", short_id(&event.run_id)))
            }
            RoderEvent::WorkflowRunStarted(event) => {
                self.update_status(&event.run_id, event.status);
                Some(format!("workflow started: {}", short_id(&event.run_id)))
            }
            RoderEvent::WorkflowRunPaused(event) => {
                self.update_status(&event.run_id, WorkflowRunStatus::Paused);
                Some(format!("workflow paused: {}", short_id(&event.run_id)))
            }
            RoderEvent::WorkflowRunResumed(event) => {
                self.update_status(&event.run_id, event.status);
                Some(format!("workflow resumed: {}", short_id(&event.run_id)))
            }
            RoderEvent::WorkflowRunStopped(event) => {
                self.update_status(&event.run_id, WorkflowRunStatus::Stopped);
                Some(format!("workflow stopped: {}", short_id(&event.run_id)))
            }
            RoderEvent::WorkflowRunCompleted(event) => {
                self.update_status(&event.run_id, WorkflowRunStatus::Completed);
                self.summaries
                    .insert(event.run_id.clone(), event.summary.clone());
                Some(format!("workflow completed: {}", short_id(&event.run_id)))
            }
            RoderEvent::WorkflowRunFailed(event) => {
                self.update_status(&event.run_id, WorkflowRunStatus::Failed);
                if let Some(summary) = &event.summary {
                    self.summaries.insert(event.run_id.clone(), summary.clone());
                }
                Some(format!("workflow failed: {}", short_id(&event.run_id)))
            }
            RoderEvent::WorkflowPhaseStarted(event) => {
                self.upsert_phase(&event.run_id, event.phase.clone());
                None
            }
            RoderEvent::WorkflowPhaseCompleted(event) => {
                self.upsert_phase(&event.run_id, event.phase.clone());
                None
            }
            RoderEvent::WorkflowAgentQueued(event) => {
                self.upsert_agent(&event.run_id, event.agent.clone());
                None
            }
            RoderEvent::WorkflowAgentStarted(event) => {
                self.upsert_agent(&event.run_id, event.agent.clone());
                None
            }
            RoderEvent::WorkflowAgentCompleted(event) => {
                self.upsert_agent(&event.run_id, event.agent.clone());
                None
            }
            RoderEvent::WorkflowAgentFailed(event) => {
                self.upsert_agent(&event.run_id, event.agent.clone());
                Some(format!(
                    "workflow agent failed: {} {}",
                    short_id(&event.run_id),
                    event.agent.agent_id
                ))
            }
            RoderEvent::WorkflowOutputRecorded(_) | RoderEvent::WorkflowCheckpointRecorded(_) => {
                None
            }
            _ => None,
        }
    }

    pub(super) fn key_action(&mut self, key: KeyEvent) -> Option<WorkflowUiAction> {
        if let Some(approval) = self.approval.as_mut() {
            return match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    Some(WorkflowUiAction::Approve(WorkflowApprovalDecision::RunOnce))
                }
                KeyCode::Char('a') | KeyCode::Char('A') => Some(WorkflowUiAction::Approve(
                    WorkflowApprovalDecision::AlwaysForScriptAndWorkspace,
                )),
                KeyCode::Char('v') | KeyCode::Char('V') => {
                    approval.show_script = !approval.show_script;
                    Some(WorkflowUiAction::ToggleScript)
                }
                KeyCode::Char('e') | KeyCode::Char('E') => Some(WorkflowUiAction::EditPrompt),
                KeyCode::Esc
                | KeyCode::Char('n')
                | KeyCode::Char('N')
                | KeyCode::Char('d')
                | KeyCode::Char('D') => Some(WorkflowUiAction::Deny),
                _ => None,
            };
        }

        match self.panel {
            WorkflowPanel::Hidden => None,
            WorkflowPanel::List | WorkflowPanel::Detail(_) => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => Some(WorkflowUiAction::Close),
                KeyCode::Backspace | KeyCode::Char('b') => Some(WorkflowUiAction::Back),
                KeyCode::Char('r') => Some(WorkflowUiAction::RefreshList),
                KeyCode::Down | KeyCode::Char('j') => Some(WorkflowUiAction::MoveSelection(1)),
                KeyCode::Up | KeyCode::Char('k') => Some(WorkflowUiAction::MoveSelection(-1)),
                KeyCode::Enter => Some(WorkflowUiAction::OpenSelected),
                KeyCode::Char('p') => Some(WorkflowUiAction::PauseSelected),
                KeyCode::Char('u') => Some(WorkflowUiAction::ResumeSelected),
                KeyCode::Char('s') => Some(WorkflowUiAction::StopSelected),
                KeyCode::Char('v') => Some(WorkflowUiAction::SaveSelected),
                KeyCode::Char('x') if matches!(self.panel, WorkflowPanel::Detail(_)) => {
                    Some(WorkflowUiAction::RestartSelectedAgent)
                }
                _ => None,
            },
        }
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        if let WorkflowPanel::Detail(run_id) = &self.panel {
            let count = self.detail_item_count(run_id);
            self.detail_selected = wrap_index(self.detail_selected, count, delta);
            return;
        }
        let count = self.run_summaries().len();
        self.selected = wrap_index(self.selected, count, delta);
    }

    pub(super) fn selected_run_id(&self) -> Option<WorkflowRunId> {
        match &self.panel {
            WorkflowPanel::Detail(run_id) => Some(run_id.clone()),
            WorkflowPanel::List | WorkflowPanel::Hidden => self
                .run_summaries()
                .get(self.selected)
                .map(|summary| summary.run_id.clone()),
        }
    }

    pub(super) fn selected_agent_id(&self) -> Option<(WorkflowRunId, String)> {
        let WorkflowPanel::Detail(run_id) = &self.panel else {
            return None;
        };
        let run = self.runs.get(run_id)?;
        let agent_index = self.detail_selected.checked_sub(run.phases.len())?;
        run.agents
            .get(agent_index)
            .map(|agent| (run_id.clone(), agent.agent_id.clone()))
    }

    pub(super) fn close_panel(&mut self) {
        self.panel = WorkflowPanel::Hidden;
    }

    pub(super) fn back_panel(&mut self) {
        self.panel = match self.panel.clone() {
            WorkflowPanel::Detail(_) => WorkflowPanel::List,
            WorkflowPanel::List | WorkflowPanel::Hidden => WorkflowPanel::Hidden,
        };
    }

    pub(crate) fn progress_panel(&self, area: Rect, theme: Theme) -> Paragraph<'static> {
        super::progress::progress_panel(self, area, theme)
    }

    pub(crate) fn trigger_line(&self, theme: Theme) -> Paragraph<'static> {
        render::trigger_line(theme)
    }

    pub(crate) fn render_overlay(&self, f: &mut Frame<'_>, area: Rect, theme: Theme) {
        render::render_overlay(self, f, area, theme);
    }

    pub(super) fn active_run(&self) -> Option<&WorkflowRun> {
        self.runs.values().find(|run| !terminal_status(run.status))
    }

    pub(super) fn run_summaries(&self) -> Vec<WorkflowRunSummary> {
        let mut runs = self.summaries.values().cloned().collect::<Vec<_>>();
        runs.sort_by(|a, b| a.run_id.cmp(&b.run_id));
        runs
    }

    fn detail_item_count(&self, run_id: &str) -> usize {
        self.runs
            .get(run_id)
            .map(|run| run.phases.len() + run.agents.len())
            .unwrap_or_default()
    }

    fn update_status(&mut self, run_id: &str, status: WorkflowRunStatus) {
        if let Some(run) = self.runs.get_mut(run_id) {
            run.status = status;
            self.summaries
                .insert(run_id.to_string(), summary_from_run(run));
        } else if let Some(summary) = self.summaries.get_mut(run_id) {
            summary.status = status;
        }
    }

    fn upsert_phase(&mut self, run_id: &str, phase: WorkflowPhase) {
        if let Some(run) = self.runs.get_mut(run_id) {
            if let Some(existing) = run
                .phases
                .iter_mut()
                .find(|existing| existing.phase_id == phase.phase_id)
            {
                *existing = phase;
            } else {
                run.phases.push(phase);
            }
            self.summaries
                .insert(run_id.to_string(), summary_from_run(run));
        }
    }

    fn upsert_agent(&mut self, run_id: &str, agent: WorkflowAgentRun) {
        if let Some(run) = self.runs.get_mut(run_id) {
            if let Some(existing) = run
                .agents
                .iter_mut()
                .find(|existing| existing.agent_id == agent.agent_id)
            {
                *existing = agent;
            } else {
                run.agents.push(agent);
            }
            self.summaries
                .insert(run_id.to_string(), summary_from_run(run));
        }
    }
}

fn wrap_index(index: usize, count: usize, delta: isize) -> usize {
    if count == 0 {
        0
    } else {
        (index as isize + delta).rem_euclid(count as isize) as usize
    }
}

fn summary_from_run(run: &WorkflowRun) -> WorkflowRunSummary {
    run.summary.clone().unwrap_or_else(|| WorkflowRunSummary {
        run_id: run.run_id.clone(),
        status: run.status,
        title: run.script.name.clone(),
        phase_count: run.phases.len() as u32,
        completed_phase_count: run
            .phases
            .iter()
            .filter(|phase| phase.completed_at.is_some())
            .count() as u32,
        agent_count: run.agents.len() as u32,
        completed_agent_count: run
            .agents
            .iter()
            .filter(|agent| agent.completed_at.is_some())
            .count() as u32,
        failed_agent_count: run
            .agents
            .iter()
            .filter(|agent| agent.error.is_some())
            .count() as u32,
        concurrency_peak: run.limits.max_concurrent_agents,
        usage: None,
        elapsed_ms: None,
        report_preview: None,
    })
}

fn contains_workflow_trigger(input: &str) -> bool {
    input
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
        .any(|word| word.eq_ignore_ascii_case("workflow"))
}

pub(super) fn terminal_status(status: WorkflowRunStatus) -> bool {
    matches!(
        status,
        WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Stopped
    )
}
