use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
};

use roder_tui_util::scroll_accel::{ScrollAccelState, ScrollDirection, ScrollSettings};
use super::tool_timeline::ToolDetail;
use super::{Theme, centered_rect};

#[derive(Debug, Clone)]
pub(super) struct ToolDetailModal {
    detail: ToolDetail,
    scroll: u16,
    wrap: bool,
    scroll_accel: ScrollAccelState,
}

impl ToolDetailModal {
    pub(super) fn new(detail: ToolDetail, scroll_settings: ScrollSettings) -> Self {
        Self {
            detail,
            scroll: 0,
            wrap: false,
            scroll_accel: ScrollAccelState::new(scroll_settings),
        }
    }

    pub(super) fn tool_id(&self) -> Option<&str> {
        self.detail.tool_id.as_deref()
    }

    pub(super) fn update_detail(&mut self, detail: ToolDetail) {
        self.detail = detail;
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> ToolDetailAction {
        match key.code {
            KeyCode::Esc => ToolDetailAction::Close,
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_by(1);
                ToolDetailAction::Handled
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_by(-1);
                ToolDetailAction::Handled
            }
            KeyCode::PageDown => {
                self.scroll_by(12);
                ToolDetailAction::Handled
            }
            KeyCode::PageUp => {
                self.scroll_by(-12);
                ToolDetailAction::Handled
            }
            KeyCode::Home => {
                self.scroll = 0;
                ToolDetailAction::Handled
            }
            KeyCode::Char('w') | KeyCode::Char('W') => {
                self.wrap = !self.wrap;
                ToolDetailAction::Handled
            }
            _ => ToolDetailAction::Handled,
        }
    }

    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.scroll_by_wheel(ScrollDirection::Down);
                true
            }
            MouseEventKind::ScrollUp => {
                self.scroll_by_wheel(ScrollDirection::Up);
                true
            }
            _ => true,
        }
    }

    fn scroll_by(&mut self, amount: i16) {
        self.scroll_accel.reset();
        let next = i32::from(self.scroll) + i32::from(amount);
        self.scroll = next.max(0).min(i32::from(u16::MAX)) as u16;
    }

    fn scroll_by_wheel(&mut self, direction: ScrollDirection) {
        let rows = self
            .scroll_accel
            .tick(direction, std::time::Instant::now())
            .min(i16::MAX as isize) as i16;
        let signed_rows = match direction {
            ScrollDirection::Down => rows,
            ScrollDirection::Up => -rows,
        };
        let next = i32::from(self.scroll) + i32::from(signed_rows);
        self.scroll = next.max(0).min(i32::from(u16::MAX)) as u16;
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum ToolDetailAction {
    Close,
    Handled,
}

pub(super) fn render_tool_detail_modal(
    f: &mut Frame<'_>,
    area: Rect,
    modal: &ToolDetailModal,
    theme: Theme,
) {
    let modal_area = tool_modal_area(area);
    f.render_widget(Clear, modal_area);

    let borders = if theme.borders_visible {
        Borders::ALL
    } else {
        Borders::NONE
    };
    let block = Block::default()
        .borders(borders)
        .border_type(theme.border_type)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .padding(Padding::horizontal(2))
        .title(Span::styled(" shell output  [x] ", theme.accent()));
    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    f.render_widget(command_line(&modal.detail, theme), chunks[0]);
    f.render_widget(context_line(&modal.detail, theme), chunks[1]);
    f.render_widget(output_text(modal, theme), chunks[2]);
    f.render_widget(controls_line(theme), chunks[3]);
}

fn tool_modal_area(area: Rect) -> Rect {
    centered_rect(
        area,
        area.width.saturating_sub(8).max(area.width.min(1)),
        area.height.saturating_sub(6).max(area.height.min(1)),
    )
}

fn command_line(detail: &ToolDetail, theme: Theme) -> Paragraph<'static> {
    let command = detail
        .command
        .as_deref()
        .filter(|command| !command.trim().is_empty())
        .unwrap_or(&detail.title);
    Paragraph::new(Line::from(vec![
        Span::styled("$ ", theme.subtle()),
        Span::styled(command.to_string(), theme.shell()),
    ]))
    .style(theme.dialog_surface())
}

fn context_line(detail: &ToolDetail, theme: Theme) -> Paragraph<'static> {
    let status = if detail.failed {
        "failed"
    } else if detail.running {
        "running"
    } else {
        "completed"
    };
    Paragraph::new(Line::from(vec![
        Span::styled(status.to_string(), theme.muted()),
        Span::styled("  ", theme.dialog_surface()),
        Span::styled(detail.title.clone(), theme.muted()),
    ]))
    .style(theme.dialog_surface())
}

fn output_text(modal: &ToolDetailModal, theme: Theme) -> Paragraph<'static> {
    let paragraph = Paragraph::new(Text::from(detail_lines(&modal.detail, theme)))
        .style(theme.dialog_surface())
        .scroll((modal.scroll, 0));
    if modal.wrap {
        paragraph.wrap(Wrap { trim: false })
    } else {
        paragraph
    }
}

fn detail_lines(detail: &ToolDetail, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !detail.arguments.trim().is_empty() {
        lines.push(Line::from(Span::styled("Arguments", theme.accent_soft())));
        lines.extend(
            detail
                .arguments
                .lines()
                .map(|line| Line::from(Span::styled(line.to_string(), theme.muted()))),
        );
        lines.push(Line::raw(""));
    }

    lines.push(Line::from(Span::styled("Output", theme.accent_soft())));
    match detail
        .output
        .as_deref()
        .filter(|output| !output.trim().is_empty())
    {
        Some(output) => lines.extend(
            output
                .lines()
                .map(|line| Line::from(Span::styled(line.to_string(), theme.text()))),
        ),
        None => lines.push(Line::from(Span::styled("(no output yet)", theme.muted()))),
    }
    lines
}

fn controls_line(theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        key_hint("Esc", theme),
        Span::styled(":close   ", theme.muted()),
        key_hint("j/k", theme),
        Span::styled(":scroll   ", theme.muted()),
        key_hint("PgUp/PgDn", theme),
        Span::styled(":page   ", theme.muted()),
        key_hint("w", theme),
        Span::styled(":wrap", theme.muted()),
    ]))
    .style(theme.dialog_surface())
}

fn key_hint(label: &'static str, theme: Theme) -> Span<'static> {
    Span::styled(label.to_string(), theme.dialog_key())
}

#[cfg(test)]
pub(super) fn detail_lines_for_test(modal: &ToolDetailModal, theme: Theme) -> Vec<String> {
    detail_lines(&modal.detail, theme)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_lines_include_arguments_and_aggregated_output() {
        let detail = ToolDetail {
            tool_id: Some("call_1".to_string()),
            title: "Shell command: echo hi".to_string(),
            command: Some("echo hi".to_string()),
            arguments: "{\"command\":\"echo hi\"}".to_string(),
            output: Some("Exit code: 0\nOutput:\nhi".to_string()),
            failed: false,
            running: false,
        };

        let lines = detail_lines(&detail, Theme::for_dark_background(true))
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(lines.iter().any(|line| line == "Arguments"));
        assert!(lines.iter().any(|line| line.contains("echo hi")));
        assert!(lines.iter().any(|line| line == "Output"));
        assert!(lines.iter().any(|line| line == "hi"));
    }
}
