use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::super::Theme;
use super::markdown::markdown_lines;
use super::patch_preview::tool_diff_preview_lines;
use super::{TimelineItem, TimelineItemKind, ToolTimelineStatus, ToolTimelineTool};

impl TimelineItem {
    pub(super) fn render(
        &self,
        selected: bool,
        expanded: bool,
        theme: Theme,
        width: u16,
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
            TimelineItemKind::Assistant { text, phase } => {
                if phase
                    .as_deref()
                    .is_some_and(|phase| !phase.is_empty() && phase != "final_answer")
                {
                    push_markdown_body_lines(
                        lines,
                        format!("  {} ", phase.as_deref().unwrap()),
                        text,
                        theme.accent_soft(),
                        item_style(theme.muted(), selected, theme),
                        theme,
                    );
                } else {
                    push_markdown_body_lines(
                        lines,
                        "",
                        text,
                        theme.subtle(),
                        item_style(theme.text(), selected, theme),
                        theme,
                    );
                }
            }
            TimelineItemKind::Reasoning(text) => push_body_lines(
                lines,
                "Thinking: ",
                text,
                theme.accent_soft().add_modifier(Modifier::ITALIC),
                item_style(
                    theme.muted().add_modifier(Modifier::ITALIC),
                    selected,
                    theme,
                ),
            ),
            TimelineItemKind::System(text) => push_body_lines(
                lines,
                "    ",
                text,
                theme.subtle(),
                item_style(theme.muted(), selected, theme),
            ),
            TimelineItemKind::TurnCompleted(summary) => {
                let left = "    Turn completed.";
                let right = format!(
                    "{}  in {}  out {}  session {} tokens",
                    format_duration(summary.elapsed),
                    format_compact_count(u64::from(summary.input_tokens)),
                    format_compact_count(u64::from(summary.output_tokens)),
                    format_compact_count(summary.session_tokens),
                );
                push_aligned_line(
                    lines,
                    left,
                    &right,
                    item_style(theme.muted(), selected, theme),
                    width,
                );
            }
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

        lines.extend(tool_diff_preview_lines(&self.entry, theme));

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
    marker: impl Into<String>,
    body: &str,
    marker_style: Style,
    body_style: Style,
) {
    let marker = marker.into();
    for (line_index, line) in body.split('\n').enumerate() {
        let marker = if line_index == 0 || marker.is_empty() {
            marker.clone()
        } else {
            "    ".to_string()
        };
        lines.push(Line::from(vec![
            Span::styled(marker, marker_style),
            Span::styled(line.to_string(), body_style),
        ]));
    }
}

fn push_markdown_body_lines(
    lines: &mut Vec<Line<'static>>,
    marker: impl Into<String>,
    body: &str,
    marker_style: Style,
    body_style: Style,
    theme: Theme,
) {
    let marker = marker.into();
    for (line_index, body_line) in markdown_lines(body, body_style, theme)
        .into_iter()
        .enumerate()
    {
        let marker = if line_index == 0 || marker.is_empty() {
            marker.clone()
        } else {
            "    ".to_string()
        };
        let mut spans = Vec::with_capacity(body_line.spans.len() + 1);
        spans.push(Span::styled(marker, marker_style));
        spans.extend(body_line.spans);
        lines.push(Line::from(spans));
    }
}

fn push_aligned_line(
    lines: &mut Vec<Line<'static>>,
    left: &str,
    right: &str,
    style: Style,
    width: u16,
) {
    let used = left.chars().count() + right.chars().count();
    let gap = usize::from(width).saturating_sub(used).max(1);
    lines.push(Line::from(vec![
        Span::styled(left.to_string(), style),
        Span::styled(" ".repeat(gap), style),
        Span::styled(right.to_string(), style),
    ]));
}

fn format_duration(duration: std::time::Duration) -> String {
    let total_millis = duration.as_millis();
    if total_millis < 1_000 {
        return format!("{}ms", total_millis);
    }

    let total_seconds = duration.as_secs();
    if total_seconds < 10 {
        return format!("{:.1} sec", total_millis as f64 / 1_000.0);
    }
    if total_seconds < 60 {
        return format!("{total_seconds} sec");
    }

    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if minutes < 60 {
        return format!("{minutes} min {seconds} sec");
    }

    let hours = minutes / 60;
    let minutes = minutes % 60;
    format!("{hours} hr {minutes} min")
}

fn format_compact_count(value: u64) -> String {
    if value < 1_000 {
        return value.to_string();
    }

    const UNITS: &[(u64, &str)] = &[(1_000_000_000, "B"), (1_000_000, "M"), (1_000, "K")];
    let (divisor, unit) = UNITS
        .iter()
        .find(|(divisor, _)| value >= *divisor)
        .copied()
        .unwrap_or((1, ""));
    let whole = value / divisor;
    let remainder = value % divisor;

    if whole >= 10 || remainder == 0 {
        return format!("{whole}{unit}");
    }

    let decimal = (remainder * 10 + divisor / 2) / divisor;
    if decimal == 10 {
        format!("{}{}", whole + 1, unit)
    } else {
        format!("{whole}.{decimal}{unit}")
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
