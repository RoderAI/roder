use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use roder_api::dynamic_workflows::{WorkflowAgentRun, WorkflowPhase, WorkflowRun};

use super::super::{Theme, centered_rect, short_id, truncate};
use super::render::status_label;
use super::state::WorkflowUiState;

pub(super) fn render_detail(
    f: &mut Frame<'_>,
    area: Rect,
    state: &WorkflowUiState,
    run_id: &str,
    theme: Theme,
) {
    let dialog_area = centered_rect(area, area.width.min(104), area.height.min(28));
    f.render_widget(Clear, dialog_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .title(Span::styled(" workflow detail ", theme.accent()));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);
    let text = state
        .runs
        .get(run_id)
        .map(|run| detail_lines(run, state.detail_selected, theme))
        .unwrap_or_else(|| Text::from(Line::from(Span::styled("Run not loaded.", theme.muted()))));
    f.render_widget(
        Paragraph::new(text)
            .style(theme.dialog_surface())
            .wrap(Wrap { trim: false }),
        inner,
    );
    render_detail_footer(f, dialog_area, theme);
}

pub(super) fn detail_lines(run: &WorkflowRun, selected: usize, theme: Theme) -> Text<'static> {
    let mut lines = vec![
        Line::from(vec![
            Span::styled(run.script.name.clone(), theme.strong()),
            Span::styled(
                format!("  {}  {}", short_id(&run.run_id), status_label(run.status)),
                theme.muted(),
            ),
        ]),
        Line::from(Span::styled(
            run.script.description.clone().unwrap_or_default(),
            theme.muted(),
        )),
        usage_line(run, theme),
        Line::from(""),
        Line::from(Span::styled("Phases", theme.accent_soft())),
    ];
    for (index, phase) in run.phases.iter().enumerate() {
        lines.push(phase_line(phase, selected == index, theme));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Agents", theme.accent_soft())));
    append_agent_rows(run, selected, theme, &mut lines);
    append_selected_detail(run, selected, theme, &mut lines);
    Text::from(lines)
}

fn usage_line(run: &WorkflowRun, theme: Theme) -> Line<'static> {
    let usage = run
        .summary
        .as_ref()
        .and_then(|summary| summary.usage.as_ref())
        .map(|usage| format!("{} tokens", usage.total_tokens))
        .unwrap_or_else(|| "tokens pending".to_string());
    let elapsed = run
        .summary
        .as_ref()
        .and_then(|summary| summary.elapsed_ms)
        .map(elapsed_label)
        .unwrap_or_else(|| "elapsed pending".to_string());
    Line::from(vec![
        Span::styled("Usage ", theme.accent_soft()),
        Span::styled(usage, theme.text()),
        Span::styled(" · ", theme.subtle()),
        Span::styled(elapsed, theme.muted()),
    ])
}

fn phase_line(phase: &WorkflowPhase, selected: bool, theme: Theme) -> Line<'static> {
    let style = if selected {
        theme.selected()
    } else {
        theme.text()
    };
    Line::from(vec![
        Span::styled("  ", theme.subtle()),
        Span::styled(
            format!("{:<10}", format!("{:?}", phase.status)),
            theme.muted(),
        ),
        Span::styled(phase.name.clone(), style),
        Span::styled(
            format!(
                "  agents queued {} done {} failed {}",
                phase.queued_agents, phase.completed_agents, phase.failed_agents
            ),
            theme.muted(),
        ),
    ])
}

fn append_agent_rows(
    run: &WorkflowRun,
    selected: usize,
    theme: Theme,
    lines: &mut Vec<Line<'static>>,
) {
    let selected_agent = selected.checked_sub(run.phases.len());
    let start = selected_agent
        .map(|index| index.saturating_sub(7))
        .unwrap_or_default();
    let end = (start + 8).min(run.agents.len());
    if start > 0 {
        lines.push(Line::from(Span::styled(
            format!("  {} earlier agents", start),
            theme.subtle(),
        )));
    }
    for (agent_index, agent) in run.agents.iter().enumerate().take(end).skip(start) {
        lines.push(agent_line(
            agent,
            selected_agent == Some(agent_index),
            theme,
        ));
    }
    if end < run.agents.len() {
        lines.push(Line::from(Span::styled(
            format!("  {} more agents", run.agents.len() - end),
            theme.subtle(),
        )));
    }
}

fn agent_line(agent: &WorkflowAgentRun, selected: bool, theme: Theme) -> Line<'static> {
    let style = if selected {
        theme.selected()
    } else {
        theme.text()
    };
    Line::from(vec![
        Span::styled("  ", theme.subtle()),
        Span::styled(
            format!("{:<10}", format!("{:?}", agent.status)),
            theme.muted(),
        ),
        Span::styled(format!("{} ", agent.agent_id), style),
        Span::styled(truncate(&agent.description, 62), theme.muted()),
    ])
}

fn append_selected_detail(
    run: &WorkflowRun,
    selected: usize,
    theme: Theme,
    lines: &mut Vec<Line<'static>>,
) {
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Selected detail",
        theme.accent_soft(),
    )));
    if let Some(phase) = run.phases.get(selected) {
        lines.push(Line::from(vec![
            Span::styled("Phase ", theme.muted()),
            Span::styled(phase.phase_id.clone(), theme.text()),
            Span::styled(format!("  {:?}", phase.status), theme.muted()),
        ]));
        if let Some(description) = &phase.description {
            lines.push(Line::from(Span::styled(
                truncate(description, 180),
                theme.text(),
            )));
        }
        return;
    }
    let Some(agent) = selected
        .checked_sub(run.phases.len())
        .and_then(|index| run.agents.get(index))
    else {
        lines.push(Line::from(Span::styled(
            "Select a phase or agent with j/k.",
            theme.muted(),
        )));
        return;
    };
    lines.push(Line::from(vec![
        Span::styled("Agent ", theme.muted()),
        Span::styled(agent.agent_id.clone(), theme.text()),
        Span::styled(format!("  phase {}", agent.phase_id), theme.muted()),
    ]));
    lines.push(Line::from(Span::styled(
        format!(
            "lane {} · model {} · trace {}",
            agent
                .lane
                .as_ref()
                .map(|lane| format!("{lane:?}"))
                .unwrap_or_else(|| "default".to_string()),
            agent.model.as_deref().unwrap_or("session"),
            trace_label(agent)
        ),
        theme.muted(),
    )));
    if let Some(usage) = &agent.usage {
        lines.push(Line::from(Span::styled(
            format!(
                "usage {} prompt + {} completion = {} total",
                usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
            ),
            theme.muted(),
        )));
    }
    if let Some(error) = &agent.error {
        lines.push(Line::from(Span::styled(
            truncate(error, 180),
            theme.error(),
        )));
    }
}

fn trace_label(agent: &WorkflowAgentRun) -> String {
    match (&agent.thread_id, &agent.turn_id) {
        (Some(thread_id), Some(turn_id)) => format!("{thread_id}/{turn_id}"),
        (Some(thread_id), None) => thread_id.clone(),
        _ => "pending".to_string(),
    }
}

fn render_detail_footer(f: &mut Frame<'_>, dialog_area: Rect, theme: Theme) {
    let footer = Rect::new(
        dialog_area.x.saturating_add(2),
        dialog_area
            .y
            .saturating_add(dialog_area.height.saturating_sub(1)),
        dialog_area.width.saturating_sub(4),
        1,
    );
    f.render_widget(
        Paragraph::new("j/k select · x restart agent · b back · p pause · u resume · s stop · v save · Esc close")
            .style(theme.muted()),
        footer,
    );
}

fn elapsed_label(ms: u64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}
