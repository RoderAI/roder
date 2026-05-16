use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::super::Theme;
use super::{TimelineItem, TimelineItemKind, ToolTimelineStatus, ToolTimelineTool};

impl TimelineItem {
    pub(super) fn render(
        &self,
        selected: bool,
        expanded: bool,
        theme: Theme,
        lines: &mut Vec<Line<'static>>,
    ) {
        match &self.kind {
            TimelineItemKind::User(text) => push_body_lines(
                lines,
                "❯ ",
                text,
                theme.accent(),
                item_style(theme.text(), selected, theme),
            ),
            TimelineItemKind::Assistant(text) => push_body_lines(
                lines,
                "    ",
                text,
                theme.subtle(),
                item_style(theme.text(), selected, theme),
            ),
            TimelineItemKind::System(text) => push_body_lines(
                lines,
                "    ",
                text,
                theme.subtle(),
                item_style(theme.muted(), selected, theme),
            ),
            TimelineItemKind::Error(text) => push_body_lines(
                lines,
                "! ",
                text,
                theme.error(),
                item_style(theme.error(), selected, theme),
            ),
            TimelineItemKind::Shell(command) => push_body_lines(
                lines,
                "$ ",
                &format!("!{command}"),
                theme.shell(),
                item_style(theme.text(), selected, theme),
            ),
            TimelineItemKind::ShellOutput(output) => push_body_lines(
                lines,
                "↳ ",
                output,
                theme.subtle(),
                item_style(theme.muted(), selected, theme),
            ),
            TimelineItemKind::Tool(tool) => {
                tool.render(selected, expanded, theme, lines);
            }
        }
    }
}

impl ToolTimelineTool {
    fn render(&self, selected: bool, expanded: bool, theme: Theme, lines: &mut Vec<Line<'static>>) {
        let marker_style = match self.status {
            ToolTimelineStatus::Running => theme.running(),
            ToolTimelineStatus::Completed => theme.tool(),
            ToolTimelineStatus::Failed => theme.error(),
        };
        let body_style = match self.status {
            ToolTimelineStatus::Failed => theme.error(),
            ToolTimelineStatus::Running => theme.text(),
            ToolTimelineStatus::Completed => theme.muted(),
        };
        let affordance = if self
            .output
            .as_ref()
            .is_some_and(|output| !output.trim().is_empty())
        {
            if expanded { "▾" } else { "▸" }
        } else {
            " "
        };
        let label = self.entry.label();
        let status = match self.status {
            ToolTimelineStatus::Running => " running",
            ToolTimelineStatus::Completed => "",
            ToolTimelineStatus::Failed => " failed",
        };
        lines.push(Line::from(vec![
            Span::styled("◆ ", marker_style),
            Span::styled(affordance.to_string(), theme.subtle()),
            Span::styled(
                format!(" {label}{status}"),
                item_style(body_style, selected, theme),
            ),
        ]));

        if expanded && let Some(output) = self.output.as_deref() {
            for line in output.lines().take(24) {
                lines.push(Line::from(vec![
                    Span::styled("  ↳ ", theme.subtle()),
                    Span::styled(
                        line.to_string(),
                        if self.status == ToolTimelineStatus::Failed {
                            theme.error()
                        } else {
                            theme.muted()
                        },
                    ),
                ]));
            }
            if output.lines().count() > 24 {
                lines.push(Line::from(vec![
                    Span::styled("  ↳ ", theme.subtle()),
                    Span::styled(
                        "output preview truncated in timeline",
                        theme.muted().add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
        }
    }
}

fn push_body_lines(
    lines: &mut Vec<Line<'static>>,
    marker: &'static str,
    body: &str,
    marker_style: Style,
    body_style: Style,
) {
    for (line_index, line) in body.split('\n').enumerate() {
        let marker = if line_index == 0 { marker } else { "    " };
        lines.push(Line::from(vec![
            Span::styled(marker, marker_style),
            Span::styled(line.to_string(), body_style),
        ]));
    }
}

fn item_style(style: Style, selected: bool, theme: Theme) -> Style {
    if selected {
        style
            .bg(theme.selection_bg)
            .fg(theme.selection_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

pub(super) fn visible_hit_rows(
    area: Rect,
    scroll: u16,
    height: u16,
    row_items: &[(usize, usize)],
) -> Vec<(u16, usize)> {
    let top = usize::from(scroll);
    let bottom = top + usize::from(height);
    row_items
        .iter()
        .filter_map(|(row, index)| {
            if *row < top || *row >= bottom {
                return None;
            }
            Some((area.y + (*row - top) as u16, *index))
        })
        .collect()
}

pub(super) fn max_scroll(row_items: &[(usize, usize)], height: u16) -> usize {
    let total_lines = row_items.last().map(|(row, _)| row + 1).unwrap_or_default();
    total_lines.saturating_sub(usize::from(height))
}
