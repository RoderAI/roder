use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use super::Theme;
use crate::roadmap::{RoadmapModeState, RoadmapPaneFocus};

#[derive(Debug, Clone)]
pub(super) struct RoadmapWorkspaceMeta {
    pub model: String,
    pub status: String,
    pub active_turn: bool,
}

pub(super) fn render_roadmap_workspace(
    f: &mut Frame<'_>,
    area: Rect,
    state: &RoadmapModeState,
    theme: Theme,
    meta: RoadmapWorkspaceMeta,
    activity: Text<'static>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);

    f.render_widget(header(state, theme, &meta), chunks[0]);
    f.render_widget(footer(theme), chunks[2]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(28),
            Constraint::Percentage(44),
            Constraint::Percentage(28),
        ])
        .split(chunks[1]);

    f.render_widget(plan_list(state, theme), body[0]);

    let middle = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(body[1]);
    f.render_widget(task_queue(state, theme), middle[0]);
    f.render_widget(selected_task(state, theme), middle[1]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(30),
            Constraint::Percentage(36),
        ])
        .split(body[2]);
    f.render_widget(agent_lanes(state, theme), right[0]);
    f.render_widget(validation_gate(state, theme), right[1]);
    f.render_widget(
        activity_pane(
            activity,
            theme,
            meta.active_turn,
            state.activity_scroll,
            state.focused_pane == RoadmapPaneFocus::Activity,
        ),
        right[2],
    );
}

fn header(
    state: &RoadmapModeState,
    theme: Theme,
    meta: &RoadmapWorkspaceMeta,
) -> Paragraph<'static> {
    let plan = state.selected_plan.as_deref().unwrap_or("select a roadmap");
    let task = state.focused_task_id.as_deref().unwrap_or("no task");
    Paragraph::new(Text::from(vec![
        Line::from(vec![
            Span::styled(" Roadmap Manager", theme.accent()),
            Span::styled("  ", theme.text()),
            Span::styled(plan.to_string(), theme.strong()),
            Span::styled("  task:", theme.muted()),
            Span::styled(task.to_string(), theme.text()),
        ]),
        Line::from(vec![
            Span::styled(" ", theme.text()),
            Span::styled(meta.status.clone(), theme.muted()),
            Span::styled("  model:", theme.muted()),
            Span::styled(meta.model.clone(), theme.text()),
        ]),
    ]))
}

fn footer(theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        Span::styled(" up/down j/k", theme.accent_soft()),
        Span::styled(" pane nav  ", theme.muted()),
        Span::styled("tab", theme.accent_soft()),
        Span::styled(" next pane  ", theme.muted()),
        Span::styled("shift-tab", theme.accent_soft()),
        Span::styled(" prev pane  ", theme.muted()),
        Span::styled("t", theme.accent_soft()),
        Span::styled(" worker  ", theme.muted()),
        Span::styled("s", theme.accent_soft()),
        Span::styled(" spawn  ", theme.muted()),
        Span::styled("e/enter", theme.accent_soft()),
        Span::styled(" execute/open  ", theme.muted()),
        Span::styled("v", theme.accent_soft()),
        Span::styled(" validate  ", theme.muted()),
        Span::styled("esc", theme.accent_soft()),
        Span::styled(" leave roadmap", theme.muted()),
    ]))
    .style(theme.text())
}

fn plan_list(state: &RoadmapModeState, theme: Theme) -> List<'static> {
    let items = if state.documents.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "no roadmap documents",
            theme.muted(),
        )))]
    } else {
        state
            .documents
            .iter()
            .take(18)
            .map(|document| {
                let selected = state
                    .selected_plan
                    .as_deref()
                    .is_some_and(|path| document.path.ends_with(path));
                let marker = if selected { ">" } else { " " };
                let style = if selected {
                    theme.selected()
                } else {
                    theme.text()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(marker, theme.accent()),
                    Span::styled(
                        format!(
                            " {}/{} ",
                            document.checked_tasks,
                            document.checked_tasks + document.unchecked_tasks
                        ),
                        theme.muted(),
                    ),
                    Span::styled(document.title.clone(), style),
                ]))
            })
            .collect()
    };
    List::new(items).block(panel(
        "Plans",
        theme,
        state.focused_pane == RoadmapPaneFocus::Plans,
    ))
}

fn task_queue(state: &RoadmapModeState, theme: Theme) -> List<'static> {
    let items = state
        .selected_document
        .as_ref()
        .map(|document| {
            document
                .tasks
                .iter()
                .take(16)
                .map(|task| {
                    let focused = state.focused_task_id.as_deref() == Some(task.id.as_str());
                    let status = task_status_label(state, task.id.as_str(), task.checked);
                    let style = if focused {
                        theme.selected()
                    } else if task.checked {
                        theme.muted()
                    } else {
                        theme.text()
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(if focused { ">" } else { " " }, theme.accent()),
                        Span::styled(format!(" {status:<8} "), status_style(theme, status)),
                        Span::styled(task.heading.clone(), style),
                    ]))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            vec![ListItem::new(Line::from(Span::styled(
                "select or create a roadmap document",
                theme.muted(),
            )))]
        });
    List::new(items).block(panel(
        "Task Queue",
        theme,
        state.focused_pane == RoadmapPaneFocus::Tasks,
    ))
}

fn selected_task(state: &RoadmapModeState, theme: Theme) -> Paragraph<'static> {
    let Some(document) = state.selected_document.as_ref() else {
        return Paragraph::new("No roadmap selected").block(panel(
            "Selected Task",
            theme,
            state.focused_pane == RoadmapPaneFocus::TaskDetail,
        ));
    };
    let Some(task) = state
        .focused_task_id
        .as_deref()
        .and_then(|id| document.tasks.iter().find(|task| task.id == id))
    else {
        return Paragraph::new("No task selected").block(panel(
            "Selected Task",
            theme,
            state.focused_pane == RoadmapPaneFocus::TaskDetail,
        ));
    };
    let workers = state
        .attached_threads
        .iter()
        .filter(|thread| thread.task_id.as_deref() == Some(task.id.as_str()))
        .count();
    let mut lines = vec![
        Line::from(Span::styled(task.heading.clone(), theme.strong())),
        Line::from(vec![
            Span::styled("id ", theme.muted()),
            Span::styled(task.id.clone(), theme.text()),
            Span::styled("  workers ", theme.muted()),
            Span::styled(workers.to_string(), theme.text()),
        ]),
    ];
    if !task.paths.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("paths ", theme.muted()),
            Span::styled(task.paths.join(", "), theme.text()),
        ]));
    }
    if let Some(run) = task.run_blocks.first() {
        lines.push(Line::from(vec![
            Span::styled("run ", theme.muted()),
            Span::styled(one_line(run), theme.shell()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Next ", theme.accent_soft()),
        Span::styled("press ", theme.muted()),
        Span::styled("s", theme.accent()),
        Span::styled(" to spawn a worker or ", theme.muted()),
        Span::styled("e", theme.accent()),
        Span::styled(" to execute through the focused thread.", theme.muted()),
    ]));
    Paragraph::new(Text::from(lines))
        .block(panel(
            "Selected Task",
            theme,
            state.focused_pane == RoadmapPaneFocus::TaskDetail,
        ))
        .scroll((state.task_detail_scroll, 0))
        .wrap(Wrap { trim: false })
}

fn agent_lanes(state: &RoadmapModeState, theme: Theme) -> List<'static> {
    let items = if state.attached_threads.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "no attached workers",
            theme.muted(),
        )))]
    } else {
        state
            .attached_threads
            .iter()
            .take(8)
            .map(|thread| {
                let selected = state.selected_thread_id.as_deref() == Some(&thread.thread_id);
                let style = if selected {
                    theme.selected()
                } else {
                    theme.text()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(if selected { ">" } else { " " }, theme.accent()),
                    Span::styled(format!(" {}", thread.thread_id), style),
                    Span::styled("  ", theme.muted()),
                    Span::styled(
                        thread.task_id.clone().unwrap_or_else(|| "-".to_string()),
                        theme.muted(),
                    ),
                ]))
            })
            .collect()
    };
    List::new(items).block(panel(
        "Agent Lanes",
        theme,
        state.focused_pane == RoadmapPaneFocus::Agents,
    ))
}

fn validation_gate(state: &RoadmapModeState, theme: Theme) -> Paragraph<'static> {
    let lines = if state.validation_diagnostics.is_empty() {
        vec![Line::from(Span::styled("ok", theme.accent_soft()))]
    } else {
        state
            .validation_diagnostics
            .iter()
            .take(6)
            .map(|diagnostic| {
                Line::from(vec![
                    Span::styled(format!("{:?} ", diagnostic.severity), theme.error()),
                    Span::styled(diagnostic.message.clone(), theme.text()),
                ])
            })
            .collect()
    };
    Paragraph::new(Text::from(lines))
        .block(panel(
            "Validation",
            theme,
            state.focused_pane == RoadmapPaneFocus::Validation,
        ))
        .scroll((state.validation_scroll, 0))
        .wrap(Wrap { trim: false })
}

fn activity_pane(
    mut activity: Text<'static>,
    theme: Theme,
    active_turn: bool,
    scroll: u16,
    focused: bool,
) -> Paragraph<'static> {
    if activity.lines.is_empty() {
        activity.lines.push(Line::from(Span::styled(
            if active_turn {
                "worker activity will stream here"
            } else {
                "no worker activity yet"
            },
            theme.muted(),
        )));
    }
    Paragraph::new(activity)
        .style(theme.text())
        .block(panel("Worker Activity", theme, focused))
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false })
}

fn panel(title: &'static str, theme: Theme, focused: bool) -> Block<'static> {
    let title_style = if focused {
        theme.accent()
    } else {
        theme.accent_soft()
    };
    let border_style = if focused {
        theme.accent()
    } else {
        theme.border()
    };
    let block = Block::default()
        .title(Span::styled(format!(" {title} "), title_style))
        .style(theme.text())
        .border_style(border_style)
        .border_type(theme.border_type);
    if theme.borders_visible {
        block.borders(Borders::ALL)
    } else {
        block
    }
}

fn task_status_label(state: &RoadmapModeState, task_id: &str, checked: bool) -> &'static str {
    if checked {
        "done"
    } else if state
        .attached_threads
        .iter()
        .any(|thread| thread.task_id.as_deref() == Some(task_id))
    {
        "assigned"
    } else if state.focused_task_id.as_deref() == Some(task_id) {
        "ready"
    } else {
        "pending"
    }
}

fn status_style(theme: Theme, status: &str) -> Style {
    match status {
        "done" => theme.muted(),
        "assigned" => theme.running(),
        "ready" => theme.accent(),
        _ => theme.subtle().add_modifier(Modifier::DIM),
    }
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
