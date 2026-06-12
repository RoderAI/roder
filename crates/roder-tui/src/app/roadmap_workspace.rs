use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use roder_roadmap::DiagnosticSeverity;

use super::Theme;
use crate::roadmap::{RoadmapModeState, RoadmapPaneFocus};

mod chrome;
mod rows;

use chrome::{footer, header};
use rows::{
    clip, list_window, one_line, overflow_row, plan_display_title, task_status, worker_row,
    workers_for_task,
};

#[derive(Debug, Clone)]
pub(super) struct RoadmapWorkspaceMeta {
    pub model: String,
    pub status: String,
    pub active_turn: bool,
    pub spinner: String,
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
            Constraint::Length(1),
        ])
        .split(area);

    f.render_widget(header(state, theme, &meta, chunks[0].width), chunks[0]);
    f.render_widget(footer(theme, chunks[2].width), chunks[2]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(26),
            Constraint::Percentage(44),
            Constraint::Percentage(30),
        ])
        .split(chunks[1]);

    f.render_widget(plan_list(state, theme, body[0]), body[0]);

    let middle = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(body[1]);
    f.render_widget(task_queue(state, theme, middle[0]), middle[0]);
    f.render_widget(selected_task(state, theme, middle[1]), middle[1]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(24),
            Constraint::Percentage(42),
        ])
        .split(body[2]);
    f.render_widget(agent_lanes(state, theme, right[0]), right[0]);
    f.render_widget(validation_gate(state, theme), right[1]);
    f.render_widget(
        activity_pane(
            activity,
            theme,
            &meta,
            state.activity_scroll,
            state.focused_pane == RoadmapPaneFocus::Activity,
        ),
        right[2],
    );
}

fn plan_list(state: &RoadmapModeState, theme: Theme, area: Rect) -> List<'static> {
    let inner_w = usize::from(area.width.saturating_sub(2));
    let rows = usize::from(area.height.saturating_sub(2));
    let items = if state.documents.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "no roadmap documents",
            theme.muted(),
        )))]
    } else {
        let selected = state
            .documents
            .iter()
            .position(|document| {
                state
                    .selected_plan
                    .as_deref()
                    .is_some_and(|path| document.path.ends_with(path))
            })
            .unwrap_or(0);
        let window = list_window(state.documents.len(), selected, rows);
        let mut items = Vec::with_capacity(rows);
        if window.clipped_above > 0 {
            items.push(overflow_row(window.clipped_above, "above", theme));
        }
        for (index, document) in state
            .documents
            .iter()
            .enumerate()
            .skip(window.start)
            .take(window.end - window.start)
        {
            let total = document.checked_tasks + document.unchecked_tasks;
            let glyph = if total > 0 && document.unchecked_tasks == 0 {
                "✓"
            } else if document.checked_tasks > 0 {
                "◐"
            } else {
                "·"
            };
            let glyph_style = if total > 0 && document.unchecked_tasks == 0 {
                theme.accent_soft()
            } else if document.checked_tasks > 0 {
                theme.running()
            } else {
                theme.subtle()
            };
            let counts = format!("{}/{}", document.checked_tasks, total);
            let title_width = inner_w.saturating_sub(counts.chars().count() + 4).max(4);
            let style = if index == selected {
                theme.selected()
            } else if total > 0 && document.unchecked_tasks == 0 {
                theme.muted()
            } else {
                theme.text()
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(if index == selected { ">" } else { " " }, theme.accent()),
                Span::styled(format!("{glyph} "), glyph_style),
                Span::styled(
                    format!(
                        "{:<title_width$}",
                        clip(&plan_display_title(document), title_width)
                    ),
                    style,
                ),
                Span::styled(format!(" {counts}"), theme.muted()),
            ])));
        }
        if window.clipped_below > 0 {
            items.push(overflow_row(window.clipped_below, "below", theme));
        }
        items
    };
    let title = format!("Plans · {}", state.documents.len());
    List::new(items).style(theme.dialog_surface()).block(panel(
        title,
        theme,
        state.focused_pane == RoadmapPaneFocus::Plans,
    ))
}

fn task_queue(state: &RoadmapModeState, theme: Theme, area: Rect) -> List<'static> {
    let rows = usize::from(area.height.saturating_sub(2));
    let (items, title) = match state.selected_document.as_ref() {
        Some(document) if !document.tasks.is_empty() => {
            let focused = state
                .focused_task_id
                .as_deref()
                .and_then(|id| document.tasks.iter().position(|task| task.id == id))
                .unwrap_or(0);
            let window = list_window(document.tasks.len(), focused, rows);
            let mut items = Vec::with_capacity(rows);
            if window.clipped_above > 0 {
                items.push(overflow_row(window.clipped_above, "above", theme));
            }
            for (index, task) in document
                .tasks
                .iter()
                .enumerate()
                .skip(window.start)
                .take(window.end - window.start)
            {
                let is_focused = index == focused;
                let status = task_status(state, task.id.as_str(), task.checked, is_focused);
                let workers = workers_for_task(state, task.id.as_str());
                let heading_style = if is_focused {
                    theme.selected()
                } else if task.checked {
                    theme.muted()
                } else {
                    theme.text()
                };
                let mut spans = vec![
                    Span::styled(if is_focused { ">" } else { " " }, theme.accent()),
                    Span::styled(format!("{} ", status.glyph), status.style(theme)),
                    Span::styled(format!("{:<8} ", status.label), status.style(theme)),
                    Span::styled(task.heading.clone(), heading_style),
                ];
                if workers > 0 {
                    spans.push(Span::styled(format!("  ●{workers}"), theme.running()));
                }
                items.push(ListItem::new(Line::from(spans)));
            }
            if window.clipped_below > 0 {
                items.push(overflow_row(window.clipped_below, "below", theme));
            }
            let done = document.tasks.iter().filter(|task| task.checked).count();
            (items, format!("Tasks · {done}/{}", document.tasks.len()))
        }
        _ => (
            vec![ListItem::new(Line::from(Span::styled(
                "select or create a roadmap document",
                theme.muted(),
            )))],
            "Tasks".to_string(),
        ),
    };
    List::new(items).style(theme.dialog_surface()).block(panel(
        title,
        theme,
        state.focused_pane == RoadmapPaneFocus::Tasks,
    ))
}

fn selected_task(state: &RoadmapModeState, theme: Theme, area: Rect) -> Paragraph<'static> {
    let inner_w = usize::from(area.width.saturating_sub(2));
    let focused = state.focused_pane == RoadmapPaneFocus::TaskDetail;
    let task = state.selected_document.as_ref().and_then(|document| {
        state
            .focused_task_id
            .as_deref()
            .and_then(|id| document.tasks.iter().find(|task| task.id == id))
    });
    let Some(task) = task else {
        return Paragraph::new(Line::from(Span::styled("No task selected", theme.muted())))
            .style(theme.dialog_surface())
            .block(panel("Task".to_string(), theme, focused));
    };
    let workers = workers_for_task(state, task.id.as_str());
    let status = task_status(state, task.id.as_str(), task.checked, true);
    let mut lines = vec![
        Line::from(Span::styled(task.heading.clone(), theme.strong())),
        Line::from(vec![
            Span::styled(
                format!("{} {}", status.glyph, status.label),
                status.style(theme),
            ),
            Span::styled("  workers ", theme.muted()),
            Span::styled(
                workers.to_string(),
                if workers > 0 {
                    theme.running()
                } else {
                    theme.text()
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("id    ", theme.accent_soft()),
            Span::styled(
                clip(&task.id, inner_w.saturating_sub(6).max(8)),
                theme.subtle(),
            ),
        ]),
    ];
    if !task.paths.is_empty() {
        let shown = task.paths.iter().take(3).cloned().collect::<Vec<_>>();
        let extra = task.paths.len().saturating_sub(shown.len());
        let mut suffix = shown.join(", ");
        if extra > 0 {
            suffix.push_str(&format!(" (+{extra} more)"));
        }
        lines.push(Line::from(vec![
            Span::styled("paths ", theme.accent_soft()),
            Span::styled(suffix, theme.text()),
        ]));
    }
    if let Some(run) = task.run_blocks.first() {
        lines.push(Line::from(vec![
            Span::styled("run   ", theme.accent_soft()),
            Span::styled(one_line(run), theme.shell()),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(" s ", theme.dialog_key()),
        Span::styled(" spawn worker  ", theme.muted()),
        Span::styled(" e ", theme.dialog_key()),
        Span::styled(" execute  ", theme.muted()),
        Span::styled("evidence required to mark done", theme.subtle()),
    ]));
    Paragraph::new(Text::from(lines))
        .style(theme.dialog_surface())
        .block(panel(format!("Task · {}", status.label), theme, focused))
        .scroll((state.task_detail_scroll, 0))
        .wrap(Wrap { trim: false })
}

fn agent_lanes(state: &RoadmapModeState, theme: Theme, area: Rect) -> List<'static> {
    let inner_w = usize::from(area.width.saturating_sub(2));
    let rows = usize::from(area.height.saturating_sub(2));
    let items = if state.attached_threads.is_empty() {
        vec![
            ListItem::new(Line::from(Span::styled("no workers yet", theme.muted()))),
            ListItem::new(Line::from(vec![
                Span::styled(" s ", theme.dialog_key()),
                Span::styled(" delegates a task", theme.subtle()),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled(" S ", theme.dialog_key()),
                Span::styled(" fans out workers", theme.subtle()),
            ])),
        ]
    } else {
        let selected = state
            .attached_threads
            .iter()
            .position(|thread| {
                state.selected_thread_id.as_deref() == Some(thread.thread_id.as_str())
            })
            .unwrap_or(0);
        let window = list_window(state.attached_threads.len(), selected, rows);
        let mut items = Vec::with_capacity(rows);
        if window.clipped_above > 0 {
            items.push(overflow_row(window.clipped_above, "above", theme));
        }
        let last_visible = window.end.saturating_sub(1);
        for (index, thread) in state
            .attached_threads
            .iter()
            .enumerate()
            .skip(window.start)
            .take(window.end - window.start)
        {
            items.push(worker_row(
                thread,
                index == selected,
                index == last_visible && window.clipped_below == 0,
                inner_w,
                theme,
            ));
        }
        if window.clipped_below > 0 {
            items.push(overflow_row(window.clipped_below, "below", theme));
        }
        items
    };
    let title = format!("Workers · {}", state.attached_threads.len());
    List::new(items).style(theme.dialog_surface()).block(panel(
        title,
        theme,
        state.focused_pane == RoadmapPaneFocus::Agents,
    ))
}

fn validation_gate(state: &RoadmapModeState, theme: Theme) -> Paragraph<'static> {
    let issues = state.validation_diagnostics.len();
    let lines = if issues == 0 {
        vec![Line::from(vec![
            Span::styled("✓ ", theme.accent_soft()),
            Span::styled("structure ok", theme.muted()),
        ])]
    } else {
        state
            .validation_diagnostics
            .iter()
            .take(8)
            .map(|diagnostic| {
                let (glyph, style) = match diagnostic.severity {
                    DiagnosticSeverity::Error => ("!", theme.error()),
                    DiagnosticSeverity::Warning => ("▲", theme.shell()),
                };
                let line = diagnostic
                    .line
                    .map(|line| format!(":{line}"))
                    .unwrap_or_default();
                Line::from(vec![
                    Span::styled(format!("{glyph}{line} "), style),
                    Span::styled(diagnostic.message.clone(), theme.text()),
                ])
            })
            .collect()
    };
    let title = if issues == 0 {
        "Validation · ✓".to_string()
    } else {
        format!("Validation · {issues}!")
    };
    Paragraph::new(Text::from(lines))
        .style(theme.dialog_surface())
        .block(panel(
            title,
            theme,
            state.focused_pane == RoadmapPaneFocus::Validation,
        ))
        .scroll((state.validation_scroll, 0))
        .wrap(Wrap { trim: false })
}

fn activity_pane(
    mut activity: Text<'static>,
    theme: Theme,
    meta: &RoadmapWorkspaceMeta,
    scroll: u16,
    focused: bool,
) -> Paragraph<'static> {
    if activity.lines.is_empty() {
        activity.lines.push(Line::from(Span::styled(
            if meta.active_turn {
                "worker activity will stream here"
            } else {
                "no worker activity yet"
            },
            theme.muted(),
        )));
    }
    let title = if meta.active_turn && !meta.spinner.is_empty() {
        format!("Activity {}", meta.spinner)
    } else {
        "Activity".to_string()
    };
    Paragraph::new(activity)
        .style(theme.dialog_surface())
        .block(panel(title, theme, focused))
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false })
}

fn panel(title: String, theme: Theme, focused: bool) -> Block<'static> {
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
        .style(theme.dialog_surface())
        .border_style(border_style)
        .border_type(theme.border_type);
    if theme.borders_visible {
        block.borders(Borders::ALL)
    } else {
        block
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, style::Color, text::Text};

    use super::*;

    fn meta() -> RoadmapWorkspaceMeta {
        RoadmapWorkspaceMeta {
            model: "test-model".to_string(),
            status: "ready".to_string(),
            active_turn: false,
            spinner: String::new(),
        }
    }

    #[test]
    fn roadmap_side_panels_paint_the_themed_surface() {
        let mut theme = Theme::for_dark_background(true);
        theme.body_background = Some(Color::Rgb(0x12, 0x34, 0x56));
        theme.dialog_bg = Color::Rgb(0x12, 0x34, 0x56);

        let state = RoadmapModeState::new(None);
        let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
        terminal
            .draw(|frame| {
                render_roadmap_workspace(
                    frame,
                    frame.area(),
                    &state,
                    theme,
                    meta(),
                    Text::default(),
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(1, 3)].style().bg,
            Some(Color::Rgb(0x12, 0x34, 0x56))
        );
        assert_eq!(
            buffer[(30, 3)].style().bg,
            Some(Color::Rgb(0x12, 0x34, 0x56))
        );
        assert_eq!(
            buffer[(65, 3)].style().bg,
            Some(Color::Rgb(0x12, 0x34, 0x56))
        );
    }
}
