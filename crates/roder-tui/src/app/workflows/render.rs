use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use roder_api::dynamic_workflows::{WorkflowCostEstimate, WorkflowRunStatus, WorkflowRunSummary};

use super::super::{Theme, centered_rect, short_id, truncate};
use super::detail::render_detail;
use super::state::{WorkflowApprovalView, WorkflowPanel, WorkflowUiState};

pub(super) fn progress_line(state: &WorkflowUiState, theme: Theme) -> Paragraph<'static> {
    let Some(run) = state.active_run() else {
        return Paragraph::new(Line::default());
    };
    let completed_agents = run
        .agents
        .iter()
        .filter(|agent| agent.completed_at.is_some())
        .count();
    Paragraph::new(Line::from(vec![
        Span::styled(" workflow ", theme.accent_soft()),
        Span::styled(
            format!("{} ", status_label(run.status)),
            status_style(run.status, theme),
        ),
        Span::styled(run.script.name.clone(), theme.text()),
        Span::styled(
            format!(
                " · phases {}/{} · agents {}/{} · {}",
                run.phases
                    .iter()
                    .filter(|phase| phase.completed_at.is_some())
                    .count(),
                run.phases.len(),
                completed_agents,
                run.agents.len(),
                short_id(&run.run_id)
            ),
            theme.muted(),
        ),
    ]))
}

pub(super) fn trigger_line(theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        Span::styled(" workflow trigger ", theme.accent_soft()),
        Span::styled("Enter plans a workflow", theme.muted()),
        Span::styled(" · ", theme.subtle()),
        Span::styled("Esc ignores once", theme.muted()),
    ]))
}

pub(super) fn render_overlay(state: &WorkflowUiState, f: &mut Frame<'_>, area: Rect, theme: Theme) {
    if let Some(approval) = &state.approval {
        render_approval(f, area, approval, theme);
        return;
    }
    match &state.panel {
        WorkflowPanel::Hidden => {}
        WorkflowPanel::List => render_list(f, area, state, theme),
        WorkflowPanel::Detail(run_id) => render_detail(f, area, state, run_id, theme),
    }
}

fn render_approval(f: &mut Frame<'_>, area: Rect, approval: &WorkflowApprovalView, theme: Theme) {
    let height = if approval.show_script { 24 } else { 15 };
    let dialog_area = centered_rect(area, area.width.min(96), area.height.min(height));
    f.render_widget(Clear, dialog_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .title(Span::styled(" workflow approval ", theme.accent()));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);
    f.render_widget(
        Paragraph::new(approval_lines(approval, theme))
            .style(theme.dialog_surface())
            .wrap(Wrap { trim: false }),
        inner,
    );
}

pub(super) fn approval_lines(approval: &WorkflowApprovalView, theme: Theme) -> Text<'static> {
    let run = &approval.run;
    let mut lines = vec![
        Line::from(vec![
            Span::styled(run.script.name.clone(), theme.strong()),
            Span::styled(format!("  {}", short_id(&run.run_id)), theme.muted()),
        ]),
        Line::from(Span::styled(
            run.script.description.clone().unwrap_or_default(),
            theme.muted(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Phases ", theme.accent_soft()),
            Span::styled(
                run.phases
                    .iter()
                    .map(|phase| phase.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                theme.text(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Scope ", theme.accent_soft()),
            Span::styled(
                "child agents only; normal policy and lane limits apply",
                theme.text(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Scale ", theme.accent_soft()),
            Span::styled(cost_label(run.cost_estimate.as_ref()), theme.text()),
        ]),
        Line::from(vec![
            Span::styled("Limits ", theme.accent_soft()),
            Span::styled(
                format!(
                    "{} concurrent, {} agents max",
                    run.limits.max_concurrent_agents, run.limits.max_agents_per_run
                ),
                theme.text(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Approval ", theme.accent_soft()),
            Span::styled(approval.approval_id.clone(), theme.subtle()),
        ]),
        Line::from(""),
        Line::from(vec![
            key_span("Enter", theme),
            Span::styled(" run once  ", theme.muted()),
            key_span("a", theme),
            Span::styled(" always  ", theme.muted()),
            key_span("v", theme),
            Span::styled(" script  ", theme.muted()),
            key_span("e", theme),
            Span::styled(" edit prompt  ", theme.muted()),
            key_span("Esc", theme),
            Span::styled(" deny", theme.muted()),
        ]),
    ];
    if approval.show_script {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Script preview",
            theme.accent_soft(),
        )));
        for line in run.script.body.as_deref().unwrap_or("").lines().take(10) {
            lines.push(Line::from(Span::styled(
                truncate(line, 100),
                theme.subtle(),
            )));
        }
    }
    Text::from(lines)
}

fn render_list(f: &mut Frame<'_>, area: Rect, state: &WorkflowUiState, theme: Theme) {
    let dialog_area = centered_rect(area, area.width.min(96), area.height.min(20));
    f.render_widget(Clear, dialog_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .title(Span::styled(" workflows ", theme.accent()));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);
    let summaries = state.run_summaries();
    let items = if summaries.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No workflow runs yet.",
            theme.muted(),
        )))]
    } else {
        summaries
            .iter()
            .enumerate()
            .map(|(index, run)| workflow_summary_row(run, index == state.selected, theme))
            .collect()
    };
    f.render_widget(List::new(items).style(theme.dialog_surface()), inner);
    render_overlay_footer(
        f,
        dialog_area,
        "Enter detail · p pause · u resume · s stop · v save · r refresh · Esc close",
        theme,
    );
}

fn workflow_summary_row(
    run: &WorkflowRunSummary,
    selected: bool,
    theme: Theme,
) -> ListItem<'static> {
    let style = if selected {
        theme.selected()
    } else {
        theme.text()
    };
    ListItem::new(Line::from(vec![
        Span::styled(
            format!("{:<9}", status_label(run.status)),
            status_style(run.status, theme),
        ),
        Span::styled(format!(" {} ", short_id(&run.run_id)), theme.muted()),
        Span::styled(run.title.clone(), style),
        Span::styled(
            format!(
                "  phases {}/{} agents {}/{}",
                run.completed_phase_count,
                run.phase_count,
                run.completed_agent_count,
                run.agent_count
            ),
            theme.muted(),
        ),
    ]))
}

fn render_overlay_footer(f: &mut Frame<'_>, dialog_area: Rect, text: &str, theme: Theme) {
    let footer = Rect::new(
        dialog_area.x.saturating_add(2),
        dialog_area
            .y
            .saturating_add(dialog_area.height.saturating_sub(1)),
        dialog_area.width.saturating_sub(4),
        1,
    );
    f.render_widget(
        Paragraph::new(text.to_string()).style(theme.muted()),
        footer,
    );
}

fn key_span(label: &str, theme: Theme) -> Span<'static> {
    Span::styled(format!(" {label} "), theme.dialog_key())
}

fn cost_label(cost: Option<&WorkflowCostEstimate>) -> String {
    match cost {
        Some(cost) => format!(
            "{}-{} child agents, ~{} prompt tokens",
            cost.min_child_agents,
            cost.max_child_agents,
            cost.estimated_prompt_tokens.unwrap_or_default()
        ),
        None => "unknown child-agent estimate".to_string(),
    }
}

pub(super) fn status_label(status: WorkflowRunStatus) -> &'static str {
    match status {
        WorkflowRunStatus::Drafted => "drafted",
        WorkflowRunStatus::AwaitingApproval => "approval",
        WorkflowRunStatus::Queued => "queued",
        WorkflowRunStatus::Running => "running",
        WorkflowRunStatus::Paused => "paused",
        WorkflowRunStatus::ApprovalWait => "waiting",
        WorkflowRunStatus::Completed => "complete",
        WorkflowRunStatus::Failed => "failed",
        WorkflowRunStatus::Stopped => "stopped",
    }
}

fn status_style(status: WorkflowRunStatus, theme: Theme) -> Style {
    match status {
        WorkflowRunStatus::Running | WorkflowRunStatus::Queued => theme.running(),
        WorkflowRunStatus::Completed => theme.accent_soft(),
        WorkflowRunStatus::Failed | WorkflowRunStatus::Stopped => theme.error(),
        WorkflowRunStatus::Paused | WorkflowRunStatus::ApprovalWait => theme.shell(),
        WorkflowRunStatus::Drafted | WorkflowRunStatus::AwaitingApproval => theme.muted(),
    }
}
