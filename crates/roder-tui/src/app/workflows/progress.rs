use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use roder_api::dynamic_workflows::{
    WorkflowAgentRun, WorkflowAgentStatus, WorkflowPhase, WorkflowPhaseStatus, WorkflowRun,
};

use super::super::{Theme, short_id, truncate};
use super::render::{status_label, status_style};
use super::state::WorkflowUiState;

const WIDE_PROGRESS_MIN_WIDTH: u16 = 96;
const WIDE_PROGRESS_MIN_HEIGHT: u16 = 24;
const WIDE_PROGRESS_HEIGHT: u16 = 11;
const SHORT_PROGRESS_HEIGHT: u16 = 9;

pub(super) fn progress_height(state: &WorkflowUiState, width: u16, frame_height: u16) -> u16 {
    if state.progress_run().is_none() {
        return 0;
    }
    if compact_progress_for_frame(width, frame_height) {
        return 1;
    }
    if frame_height < 30 {
        SHORT_PROGRESS_HEIGHT
    } else {
        WIDE_PROGRESS_HEIGHT
    }
}

pub(super) fn progress_panel(
    state: &WorkflowUiState,
    area: Rect,
    theme: Theme,
) -> Paragraph<'static> {
    if area.height <= 1 || area.width < WIDE_PROGRESS_MIN_WIDTH {
        return progress_line(state, theme);
    }
    let Some(run) = state.progress_run() else {
        return Paragraph::new(Line::default());
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type)
        .border_style(theme.border())
        .title(Span::styled(" workflow progress ", theme.accent_soft()));
    Paragraph::new(progress_panel_lines(run, area.width, area.height, theme))
        .block(block)
        .wrap(Wrap { trim: false })
}

fn progress_line(state: &WorkflowUiState, theme: Theme) -> Paragraph<'static> {
    let Some(run) = state.progress_run() else {
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

pub(super) fn progress_panel_lines(
    run: &WorkflowRun,
    width: u16,
    height: u16,
    theme: Theme,
) -> Text<'static> {
    let inner_width = usize::from(width.saturating_sub(2)).max(1);
    let phase_count = run.phases.len();
    let completed_phase_count = completed_phase_count(run);
    let completed_agent_count = completed_agent_count(run);
    let agent_count = agent_count(run);
    let elapsed = elapsed_label_for_run(run).unwrap_or_else(|| "elapsed pending".to_string());
    let mut lines = vec![
        Line::from(vec![
            Span::styled(run.script.name.clone(), theme.accent()),
            Span::styled("  ", theme.muted()),
            Span::styled(status_label(run.status), status_style(run.status, theme)),
            Span::styled(
                format!(
                    "  {} phases  {} agents  {}",
                    phase_progress_label(completed_phase_count, phase_count),
                    agent_progress_label(completed_agent_count, agent_count),
                    elapsed
                ),
                theme.muted(),
            ),
        ]),
        Line::from(Span::styled(
            truncate(
                &run.script.description.clone().unwrap_or_default(),
                inner_width,
            ),
            theme.muted(),
        )),
        Line::from(""),
    ];

    let left_width = phase_column_width(inner_width);
    let right_width = inner_width.saturating_sub(left_width + 3);
    let active_index = active_phase_index(run);
    let active_phase = active_index.and_then(|index| run.phases.get(index));
    let active_agents = agents_for_progress(run, active_phase);
    lines.push(Line::from(vec![
        Span::styled(pad_right("Phases", left_width), theme.text()),
        Span::styled(" | ", theme.border()),
        Span::styled(
            truncate(
                &active_phase_header(active_phase, active_agents.len()),
                right_width,
            ),
            theme.text(),
        ),
    ]));

    let row_count = usize::from(height.saturating_sub(6)).max(1);
    for row in 0..row_count {
        let phase = run.phases.get(row);
        let agent = active_agents.get(row).copied();
        lines.push(Line::from(vec![
            Span::styled(
                phase_cell(phase, row, active_index, left_width, run),
                phase_cell_style(phase, row, active_index, theme),
            ),
            Span::styled(" | ", theme.border()),
            Span::styled(
                agent_cell(agent, right_width),
                agent_cell_style(agent, theme),
            ),
        ]));
    }
    if active_agents.len() > row_count || run.phases.len() > row_count {
        lines.push(Line::from(vec![
            Span::styled(
                overflow_cell(run.phases.len(), row_count, left_width, "more phases"),
                theme.subtle(),
            ),
            Span::styled(" | ", theme.border()),
            Span::styled(
                overflow_cell(active_agents.len(), row_count, right_width, "more agents"),
                theme.subtle(),
            ),
        ]));
    }
    Text::from(lines)
}

fn compact_progress_for_frame(width: u16, height: u16) -> bool {
    width < WIDE_PROGRESS_MIN_WIDTH || height < WIDE_PROGRESS_MIN_HEIGHT
}

fn phase_column_width(inner_width: usize) -> usize {
    (inner_width / 4).clamp(24, 42)
}

fn active_phase_index(run: &WorkflowRun) -> Option<usize> {
    run.phases
        .iter()
        .position(|phase| phase.status == WorkflowPhaseStatus::Running)
        .or_else(|| {
            run.phases.iter().position(|phase| {
                !matches!(
                    phase.status,
                    WorkflowPhaseStatus::Completed
                        | WorkflowPhaseStatus::Failed
                        | WorkflowPhaseStatus::Skipped
                )
            })
        })
        .or_else(|| run.phases.len().checked_sub(1))
}

fn active_phase_header(phase: Option<&WorkflowPhase>, agent_count: usize) -> String {
    match phase {
        Some(phase) => format!("{} · {} agents", phase.name, agent_count),
        None => "Agents".to_string(),
    }
}

fn agents_for_progress<'a>(
    run: &'a WorkflowRun,
    phase: Option<&WorkflowPhase>,
) -> Vec<&'a WorkflowAgentRun> {
    let mut agents = match phase {
        Some(phase) => run
            .agents
            .iter()
            .filter(|agent| agent.phase_id == phase.phase_id)
            .collect::<Vec<_>>(),
        None => run.agents.iter().collect::<Vec<_>>(),
    };
    if agents.is_empty() {
        agents = run.agents.iter().collect::<Vec<_>>();
    }
    agents.sort_by_key(|agent| agent.started_at);
    agents
}

fn phase_cell(
    phase: Option<&WorkflowPhase>,
    row: usize,
    active_index: Option<usize>,
    width: usize,
    run: &WorkflowRun,
) -> String {
    let Some(phase) = phase else {
        return pad_right("", width);
    };
    let selected = active_index == Some(row);
    let prefix = phase_status_marker(phase.status, selected);
    let total = phase_agent_total(phase, run);
    let label = if selected {
        format!("{} {}", row + 1, phase.name)
    } else {
        phase.name.clone()
    };
    let text = format!(
        "{prefix} {} {}/{}",
        truncate(&label, width.saturating_sub(8)),
        phase.completed_agents,
        total
    );
    pad_right(&truncate(&text, width), width)
}

fn phase_cell_style(
    phase: Option<&WorkflowPhase>,
    row: usize,
    active_index: Option<usize>,
    theme: Theme,
) -> Style {
    let Some(phase) = phase else {
        return theme.subtle();
    };
    if active_index == Some(row) {
        return theme.accent();
    }
    match phase.status {
        WorkflowPhaseStatus::Completed => theme.accent_soft(),
        WorkflowPhaseStatus::Failed => theme.error(),
        WorkflowPhaseStatus::Running => theme.running(),
        WorkflowPhaseStatus::Queued | WorkflowPhaseStatus::Skipped => theme.muted(),
    }
}

fn agent_cell(agent: Option<&WorkflowAgentRun>, width: usize) -> String {
    let Some(agent) = agent else {
        return pad_right("", width);
    };
    let model = agent.model.as_deref().unwrap_or("model pending");
    let usage = agent
        .usage
        .as_ref()
        .map(|usage| format!("{} tok", compact_count(u64::from(usage.total_tokens))))
        .unwrap_or_else(|| "tokens pending".to_string());
    let elapsed = elapsed_label_for_agent(agent).unwrap_or_else(|| status_label_for_agent(agent));
    let text = format!(
        "{} {}  {}  {} · {}",
        agent_status_marker(agent.status),
        truncate(&agent.description, width.saturating_sub(38)),
        truncate(model, 22),
        usage,
        elapsed
    );
    pad_right(&truncate(&text, width), width)
}

fn agent_cell_style(agent: Option<&WorkflowAgentRun>, theme: Theme) -> Style {
    let Some(agent) = agent else {
        return theme.subtle();
    };
    match agent.status {
        WorkflowAgentStatus::Completed => theme.accent_soft(),
        WorkflowAgentStatus::Running => theme.running(),
        WorkflowAgentStatus::Failed | WorkflowAgentStatus::Timeout => theme.error(),
        WorkflowAgentStatus::Cancelled => theme.shell(),
        WorkflowAgentStatus::Queued => theme.muted(),
    }
}

fn phase_status_marker(status: WorkflowPhaseStatus, selected: bool) -> &'static str {
    if selected {
        return ">";
    }
    match status {
        WorkflowPhaseStatus::Completed => "✓",
        WorkflowPhaseStatus::Running => "●",
        WorkflowPhaseStatus::Failed => "!",
        WorkflowPhaseStatus::Skipped => "-",
        WorkflowPhaseStatus::Queued => "·",
    }
}

fn agent_status_marker(status: WorkflowAgentStatus) -> &'static str {
    match status {
        WorkflowAgentStatus::Completed => "✓",
        WorkflowAgentStatus::Running => "●",
        WorkflowAgentStatus::Failed | WorkflowAgentStatus::Timeout => "!",
        WorkflowAgentStatus::Cancelled => "-",
        WorkflowAgentStatus::Queued => "·",
    }
}

fn phase_agent_total(phase: &WorkflowPhase, run: &WorkflowRun) -> u32 {
    let actual = run
        .agents
        .iter()
        .filter(|agent| agent.phase_id == phase.phase_id)
        .count() as u32;
    actual
        .max(phase.queued_agents)
        .max(phase.completed_agents.saturating_add(phase.failed_agents))
}

fn completed_phase_count(run: &WorkflowRun) -> usize {
    run.summary
        .as_ref()
        .map(|summary| summary.completed_phase_count as usize)
        .unwrap_or_else(|| {
            run.phases
                .iter()
                .filter(|phase| phase.status == WorkflowPhaseStatus::Completed)
                .count()
        })
}

fn completed_agent_count(run: &WorkflowRun) -> usize {
    run.summary
        .as_ref()
        .map(|summary| summary.completed_agent_count as usize)
        .unwrap_or_else(|| {
            run.agents
                .iter()
                .filter(|agent| agent.status == WorkflowAgentStatus::Completed)
                .count()
        })
}

fn agent_count(run: &WorkflowRun) -> usize {
    run.summary
        .as_ref()
        .map(|summary| summary.agent_count as usize)
        .filter(|count| *count > 0)
        .unwrap_or_else(|| {
            run.cost_estimate
                .as_ref()
                .map(|cost| cost.max_child_agents as usize)
                .filter(|count| *count > 0)
                .unwrap_or(run.agents.len())
        })
}

fn phase_progress_label(completed: usize, total: usize) -> String {
    if total == 0 {
        "0 phases".to_string()
    } else {
        format!("{completed}/{total}")
    }
}

fn agent_progress_label(completed: usize, total: usize) -> String {
    if total == 0 {
        "0 agents".to_string()
    } else {
        format!("{completed}/{total}")
    }
}

fn elapsed_label_for_run(run: &WorkflowRun) -> Option<String> {
    run.summary
        .as_ref()
        .and_then(|summary| summary.elapsed_ms)
        .map(elapsed_label)
        .or_else(|| elapsed_between(run.started_at, run.completed_at))
}

fn elapsed_label_for_agent(agent: &WorkflowAgentRun) -> Option<String> {
    elapsed_between(agent.started_at, agent.completed_at)
}

fn elapsed_between(
    started_at: Option<time::OffsetDateTime>,
    completed_at: Option<time::OffsetDateTime>,
) -> Option<String> {
    let elapsed = completed_at? - started_at?;
    let seconds = elapsed.whole_seconds().max(0) as u64;
    Some(elapsed_label(seconds.saturating_mul(1000)))
}

fn elapsed_label(ms: u64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn compact_count(value: u64) -> String {
    if value < 1000 {
        value.to_string()
    } else if value < 100_000 {
        format!("{:.1}k", value as f64 / 1000.0)
    } else {
        format!("{}k", value / 1000)
    }
}

fn status_label_for_agent(agent: &WorkflowAgentRun) -> String {
    match agent.status {
        WorkflowAgentStatus::Running => "running".to_string(),
        WorkflowAgentStatus::Queued => "queued".to_string(),
        WorkflowAgentStatus::Completed => "done".to_string(),
        WorkflowAgentStatus::Failed => "failed".to_string(),
        WorkflowAgentStatus::Timeout => "timeout".to_string(),
        WorkflowAgentStatus::Cancelled => "cancelled".to_string(),
    }
}

fn overflow_cell(total: usize, shown: usize, width: usize, label: &str) -> String {
    if total <= shown {
        pad_right("", width)
    } else {
        pad_right(&format!("{} {label}", total - shown), width)
    }
}

fn pad_right(value: &str, width: usize) -> String {
    format!("{value:<width$}")
}
