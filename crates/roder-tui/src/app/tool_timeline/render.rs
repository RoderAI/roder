use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::super::Theme;
use super::markdown::markdown_lines;
use super::patch_preview::{tool_diff_preview, tool_diff_preview_lines};
use super::{
    TimelineItem, TimelineItemKind, ToolTimelineStatus, ToolTimelineTool, reasoning_visible_body,
};

impl TimelineItem {
    pub(super) fn render(
        &self,
        selected: bool,
        expanded: bool,
        theme: Theme,
        width: u16,
        animation_frame: u64,
        lines: &mut Vec<Line<'static>>,
    ) {
        match &self.kind {
            TimelineItemKind::User(text) => {
                push_user_block_lines(lines, text, selected, theme, width)
            }
            TimelineItemKind::Assistant { text, phase: _ } => push_markdown_body_lines(
                lines,
                "",
                text,
                theme.subtle(),
                item_style(theme.text(), selected, theme),
                theme,
            ),
            TimelineItemKind::Reasoning(text) => {
                let body = reasoning_visible_body(text);
                if !body.trim().is_empty() {
                    push_body_lines(
                        lines,
                        "Thinking: ",
                        &body,
                        theme.accent_soft().add_modifier(Modifier::ITALIC),
                        item_style(
                            theme.muted().add_modifier(Modifier::ITALIC),
                            selected,
                            theme,
                        ),
                    );
                }
            }
            TimelineItemKind::System(text) => push_body_lines(
                lines,
                "    ",
                text,
                theme.subtle(),
                item_style(theme.muted(), selected, theme),
            ),
            TimelineItemKind::TurnCompleted(summary) => {
                let reasoning = summary
                    .reasoning_tokens
                    .filter(|tokens| *tokens > 0)
                    .map(|tokens| format!("  thinking {}", format_compact_count(u64::from(tokens))))
                    .unwrap_or_default();
                let text = format!(
                    "  Turn completed in {}.  ↑ {}  ↓ {}{}  session {} tokens",
                    format_duration(summary.elapsed),
                    format_compact_count(u64::from(summary.input_tokens)),
                    format_compact_count(u64::from(summary.output_tokens)),
                    reasoning,
                    format_compact_count(summary.session_tokens),
                );
                lines.push(Line::from(Span::styled(
                    pad_to_width(&text, width),
                    item_style(theme.muted(), selected, theme),
                )));
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
                tool.render(selected, expanded, theme, animation_frame, lines);
            }
        }
    }
}

impl ToolTimelineTool {
    fn render(
        &self,
        selected: bool,
        expanded: bool,
        theme: Theme,
        animation_frame: u64,
        lines: &mut Vec<Line<'static>>,
    ) {
        let marker_style = match self.status {
            ToolTimelineStatus::Running => running_tool_marker_style(animation_frame),
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
        let diff_preview = tool_diff_preview(&self.entry);
        let label = diff_preview
            .as_ref()
            .map(|preview| preview.title())
            .unwrap_or_else(|| self.entry.label());
        let status = match self.status {
            ToolTimelineStatus::Running => " running",
            ToolTimelineStatus::Completed => "",
            ToolTimelineStatus::Failed => " failed",
        };
        lines.push(Line::from(vec![
            Span::styled("  ◆ ", marker_style),
            Span::styled(affordance.to_string(), theme.subtle()),
            Span::styled(
                format!(" {label}{status}"),
                item_style(body_style, selected, theme),
            ),
        ]));

        if let Some(preview) = diff_preview.as_ref() {
            lines.extend(tool_diff_preview_lines(preview, theme));
        }

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

pub(super) fn push_tool_overflow_line(
    hidden_count: usize,
    selected: bool,
    theme: Theme,
    width: u16,
    lines: &mut Vec<Line<'static>>,
) {
    let label = if hidden_count == 1 {
        "  › 1 more".to_string()
    } else {
        format!("  › {hidden_count} more")
    };
    lines.push(Line::from(Span::styled(
        pad_to_width(&label, width),
        item_style(
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
            selected,
            theme,
        ),
    )));
}

fn running_tool_marker_style(animation_frame: u64) -> Style {
    const PURPLE_FADE: [u8; 6] = [54, 91, 129, 135, 129, 91];
    let color = PURPLE_FADE[(animation_frame as usize) % PURPLE_FADE.len()];
    Style::default()
        .fg(ratatui::style::Color::Indexed(color))
        .add_modifier(Modifier::BOLD)
}

fn push_user_block_lines(
    lines: &mut Vec<Line<'static>>,
    body: &str,
    selected: bool,
    theme: Theme,
    width: u16,
) {
    let style = item_style(theme.user_surface(), selected, theme);
    for (line_index, line) in body.split('\n').enumerate() {
        let prefix = if line_index == 0 { "  ❯ " } else { "    " };
        let text = format!("{prefix}{line}");
        lines.push(Line::from(Span::styled(pad_to_width(&text, width), style)));
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

fn pad_to_width(text: &str, width: u16) -> String {
    let width = usize::from(width);
    let used = text.chars().count();
    if used >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - used))
    }
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

pub(super) fn max_scroll(total_lines: usize, height: u16) -> usize {
    total_lines.saturating_sub(usize::from(height))
}
