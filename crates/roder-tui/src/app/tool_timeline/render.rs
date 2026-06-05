use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use time::OffsetDateTime;

use super::super::Theme;
use super::super::stream_animation::{
    AnimatedText, StreamFadePalette, animated_markdown_lines, animated_plain_lines,
};
use super::is_shell_like_tool;
use super::markdown::markdown_lines;
use super::patch_preview::{tool_diff_preview, tool_diff_preview_lines};
use super::preview::tool_title;
use super::{
    MESSAGE_FOLD_LINE_LIMIT, RUNNING_SHELL_TAIL_ROWS, TimelineItem, TimelineItemKind,
    ToolTimelineStatus, ToolTimelineTool, reasoning_visible_body,
};

impl TimelineItem {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render(
        &self,
        selected: bool,
        expanded: bool,
        theme: Theme,
        width: u16,
        animation_frame: u64,
        message_folding: bool,
        prev_timestamp: Option<OffsetDateTime>,
        lines: &mut Vec<Line<'static>>,
    ) {
        match &self.kind {
            TimelineItemKind::User(text) => {
                let folded = fold_message_body(text, expanded || !message_folding);
                push_user_block_lines(lines, &folded.body, selected, theme, width);
                push_fold_notice(lines, folded.hidden_lines, theme, message_folding);
            }
            TimelineItemKind::Assistant(message) => {
                let rendered = message.animator.rendered_text();
                let body = if rendered.is_animating() {
                    rendered.as_str()
                } else {
                    &message.text
                };
                let body_style = assistant_body_style(message.phase.as_deref(), theme);
                let fade_palette = assistant_fade_palette(message.phase.as_deref());
                let folded = fold_message_body(body, expanded || !message_folding);
                if rendered.is_animating() {
                    let animated = message.animator.rendered_text();
                    let animated = if folded.body.len() == animated.as_str().len() {
                        animated
                    } else {
                        AnimatedText::from_visible(folded.body.clone())
                    };
                    push_rendered_body_lines(
                        lines,
                        "",
                        animated_markdown_lines(
                            &animated,
                            item_style(body_style, selected, theme),
                            theme,
                            fade_palette,
                            markdown_lines,
                        ),
                        theme.subtle(),
                    );
                } else {
                    push_markdown_body_lines(
                        lines,
                        "",
                        &folded.body,
                        theme.subtle(),
                        item_style(body_style, selected, theme),
                        theme,
                    );
                }
                push_fold_notice(lines, folded.hidden_lines, theme, message_folding);
            }
            TimelineItemKind::Reasoning(message) => {
                let rendered = message.animator.rendered_text();
                let text = if rendered.is_animating() {
                    rendered.as_str()
                } else {
                    &message.text
                };
                let body = reasoning_visible_body(text);
                if !body.trim().is_empty() {
                    let folded = fold_message_body(&body, expanded || !message_folding);
                    let max_text_width = (width as usize).saturating_sub(4);
                    let wrapped_body = wrap_text(&folded.body, max_text_width).join("\n");
                    let body_style = item_style(
                        theme.thinking().add_modifier(Modifier::ITALIC),
                        selected,
                        theme,
                    );
                    if rendered.is_animating() {
                        let vis_body = reasoning_visible_body(rendered.as_str());
                        let folded_vis_body =
                            fold_message_body(&vis_body, expanded || !message_folding);
                        let wrapped_vis_body =
                            wrap_text(&folded_vis_body.body, max_text_width).join("\n");

                        let full_body = reasoning_visible_body(&message.text);
                        let folded_full_body =
                            fold_message_body(&full_body, expanded || !message_folding);
                        let wrapped_full_body =
                            wrap_text(&folded_full_body.body, max_text_width).join("\n");

                        let pending_body = if wrapped_full_body.starts_with(&wrapped_vis_body) {
                            wrapped_full_body[wrapped_vis_body.len()..].to_string()
                        } else {
                            String::new()
                        };

                        let animated = AnimatedText::new(
                            wrapped_vis_body,
                            pending_body,
                            rendered.gradient_len,
                        );
                        push_reasoning_rendered_lines(
                            lines,
                            animated_plain_lines(
                                &animated,
                                body_style,
                                theme,
                                StreamFadePalette::Neutral,
                            ),
                            theme.thinking(),
                        );
                    } else {
                        push_reasoning_body_lines(
                            lines,
                            &wrapped_body,
                            theme.thinking(),
                            body_style,
                        );
                    }
                    push_fold_notice(lines, folded.hidden_lines, theme, message_folding);
                }
            }
            TimelineItemKind::System(text) => {
                let folded = fold_message_body(text, expanded || !message_folding);
                if !push_entrypoint_hint_lines(lines, &folded.body, selected, theme, width) {
                    push_body_lines(
                        lines,
                        "    ",
                        &folded.body,
                        theme.subtle(),
                        item_style(theme.muted(), selected, theme),
                    );
                }
                push_fold_notice(lines, folded.hidden_lines, theme, message_folding);
            }
            TimelineItemKind::TurnCompleted(summary) => {
                let reasoning = summary
                    .reasoning_tokens
                    .filter(|tokens| *tokens > 0)
                    .map(|tokens| format!("  thinking {}", format_compact_count(u64::from(tokens))))
                    .unwrap_or_default();
                let text = format!(
                    "  Turn completed in {}.  ↑ {}  ↓ {}{}  thread {} tokens",
                    format_duration(summary.elapsed),
                    format_compact_count(u64::from(summary.input_tokens)),
                    format_compact_count(u64::from(summary.output_tokens)),
                    reasoning,
                    format_compact_count(summary.thread_tokens),
                );
                lines.push(Line::from(Span::styled(
                    pad_to_width(&text, width),
                    item_style(theme.muted(), selected, theme),
                )));
            }
            TimelineItemKind::Error(text) => {
                let folded = fold_message_body(text, expanded || !message_folding);
                push_body_lines(
                    lines,
                    "! ",
                    &folded.body,
                    theme.error(),
                    item_style(theme.error(), selected, theme),
                );
                push_fold_notice(lines, folded.hidden_lines, theme, message_folding);
            }
            TimelineItemKind::Shell(command) => push_body_lines(
                lines,
                "$ ",
                &format!("!{command}"),
                theme.shell(),
                item_style(theme.text(), selected, theme),
            ),
            TimelineItemKind::ShellOutput(output) => {
                let folded = fold_message_body(output, expanded || !message_folding);
                push_body_lines(
                    lines,
                    "↳ ",
                    &folded.body,
                    theme.subtle(),
                    item_style(theme.muted(), selected, theme),
                );
                push_fold_notice(lines, folded.hidden_lines, theme, message_folding);
            }
            TimelineItemKind::Tool(tool) => {
                tool.render(
                    selected,
                    expanded,
                    theme,
                    width,
                    animation_frame,
                    prev_timestamp,
                    lines,
                );
            }
            TimelineItemKind::SubagentTrace(trace) => {
                trace.render(selected, expanded, theme, animation_frame, lines);
            }
            TimelineItemKind::PlanReview(review) => {
                review.render(selected, expanded, theme, lines);
            }
        }
    }
}

fn tail_lines(output: &str, limit: usize) -> Vec<&str> {
    let lines = output.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(limit);
    lines[start..].to_vec()
}

struct FoldedBody {
    body: String,
    hidden_lines: usize,
}

fn fold_message_body(body: &str, expanded: bool) -> FoldedBody {
    let lines = body.lines().collect::<Vec<_>>();
    if expanded || lines.len() <= MESSAGE_FOLD_LINE_LIMIT {
        return FoldedBody {
            body: body.to_string(),
            hidden_lines: 0,
        };
    }

    FoldedBody {
        body: lines[..MESSAGE_FOLD_LINE_LIMIT].join("\n"),
        hidden_lines: lines.len() - MESSAGE_FOLD_LINE_LIMIT,
    }
}

fn push_fold_notice(
    lines: &mut Vec<Line<'static>>,
    hidden_lines: usize,
    theme: Theme,
    enabled: bool,
) {
    if hidden_lines == 0 || !enabled {
        return;
    }
    let label = if hidden_lines == 1 {
        "  … 1 more line".to_string()
    } else {
        format!("  … {hidden_lines} more lines")
    };
    lines.push(Line::from(Span::styled(
        label,
        theme.muted().add_modifier(Modifier::ITALIC),
    )));
}

impl ToolTimelineTool {
    #[allow(clippy::too_many_arguments)]
    fn render(
        &self,
        selected: bool,
        expanded: bool,
        theme: Theme,
        width: u16,
        animation_frame: u64,
        prev_timestamp: Option<OffsetDateTime>,
        lines: &mut Vec<Line<'static>>,
    ) {
        // 1. Calculate symbol and style
        let (symbol, symbol_style) =
            tool_status_symbol_and_style(&self.entry.name, self.status, animation_frame, theme);

        // 2. Format tool title
        let t_title = tool_title(&self.entry.name);
        let tool_title_style = if self.status == ToolTimelineStatus::Failed {
            theme.error()
        } else if self.status == ToolTimelineStatus::Running {
            theme.text()
        } else if is_read_search_tool(&self.entry.name) {
            Style::default().fg(theme.diff_added)
        } else {
            Style::default().fg(theme.mode_plan)
        };

        // 3. Format arguments
        let formatted_args = format_tool_arguments(&self.entry.arguments);

        // 4. Calculate timing
        let time_str = format!(
            "{:02}:{:02}:{:02}",
            self.started_at.hour(),
            self.started_at.minute(),
            self.started_at.second()
        );
        let rel_str = if let Some(prev) = prev_timestamp {
            let diff = self.started_at - prev;
            format_relative_seconds(diff.whole_seconds().unsigned_abs())
        } else {
            "0s".to_string()
        };

        // 5. Layout calculation
        let right_width = 8 + 2 + rel_str.chars().count(); // HH:MM:SS  relative time

        // Left width excluding args:
        // - Cursor (2)
        // - Status symbol + space (2)
        // - Expand affordance + space (2)
        // - Tool title length
        // - Double space before args (2)
        let left_width_excluding_args = 2 + 2 + 2 + t_title.chars().count() + 2;

        let mut args_str = formatted_args;
        if left_width_excluding_args + args_str.chars().count() + right_width >= width as usize {
            let allowed_width = (width as usize)
                .saturating_sub(left_width_excluding_args + right_width)
                .saturating_sub(4); // room for "..." and a space
            if allowed_width > 0 {
                let truncated: String = args_str.chars().take(allowed_width).collect();
                args_str = format!("{}...", truncated);
            } else {
                args_str = String::new();
            }
        }

        let left_len = left_width_excluding_args + args_str.chars().count();
        let pad_len = (width as usize).saturating_sub(left_len + right_width);

        // 6. Assemble spans
        let mut spans = Vec::new();

        // Selection Background modifier
        let bg_style = if selected {
            Style::default().bg(theme.selection_bg)
        } else {
            Style::default()
        };

        // Cursor / prefix spaces
        if selected {
            spans.push(Span::styled(
                "▶ ",
                bg_style.fg(theme.mode_plan).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled("  ", bg_style));
        }

        // Status Symbol
        spans.push(Span::styled(
            format!("{} ", symbol),
            symbol_style.patch(bg_style),
        ));

        // Expand affordance
        let affordance = if self
            .output
            .as_ref()
            .is_some_and(|output| !output.trim().is_empty())
        {
            if expanded { "▼" } else { "▶" }
        } else {
            " "
        };
        let affordance_style = if selected {
            bg_style.fg(theme.selection_fg)
        } else {
            theme.subtle()
        };
        spans.push(Span::styled(
            format!("{} ", affordance),
            affordance_style.patch(bg_style),
        ));

        // Tool Title
        let title_style = if selected {
            bg_style.fg(theme.selection_fg).add_modifier(Modifier::BOLD)
        } else {
            tool_title_style.add_modifier(Modifier::BOLD)
        };
        spans.push(Span::styled(t_title, title_style));

        // Spacing before args
        spans.push(Span::styled("  ", bg_style));

        // Arguments
        let args_span_style = if selected {
            bg_style.fg(theme.selection_fg)
        } else {
            theme.muted()
        };
        spans.push(Span::styled(args_str, args_span_style));

        // Padding
        spans.push(Span::styled(" ".repeat(pad_len), bg_style));

        // Timestamp
        let ts_span_style = if selected {
            bg_style.fg(theme.selection_fg)
        } else {
            theme.muted()
        };
        spans.push(Span::styled(time_str, ts_span_style));

        // Spacing before rel
        spans.push(Span::styled("  ", bg_style));

        // Relative time
        let rel_span_style = if selected {
            bg_style.fg(theme.selection_fg)
        } else {
            theme.muted()
        };
        spans.push(Span::styled(rel_str, rel_span_style));

        lines.push(Line::from(spans));

        let diff_preview = tool_diff_preview(&self.entry);
        if let Some(preview) = diff_preview.as_ref() {
            lines.extend(tool_diff_preview_lines(preview, theme, width));
        }

        let live_tail =
            self.status == ToolTimelineStatus::Running && is_shell_like_tool(&self.entry.name);
        if (expanded || live_tail)
            && let Some(output) = self.output.as_deref()
        {
            let output_lines = if live_tail {
                tail_lines(output, RUNNING_SHELL_TAIL_ROWS)
            } else {
                output.lines().take(24).collect::<Vec<_>>()
            };
            for line in output_lines {
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

fn format_relative_seconds(total_seconds: u64) -> String {
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if minutes == 0 {
        format!("{seconds}s")
    } else if seconds == 0 {
        format!("{minutes}m")
    } else {
        format!("{minutes}m {seconds}s")
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

fn assistant_body_style(phase: Option<&str>, theme: Theme) -> Style {
    if is_commentary_phase(phase) {
        theme.commentary()
    } else {
        theme.text()
    }
}

fn assistant_fade_palette(phase: Option<&str>) -> StreamFadePalette {
    if is_commentary_phase(phase) {
        StreamFadePalette::Commentary
    } else {
        StreamFadePalette::Accent
    }
}

fn is_commentary_phase(phase: Option<&str>) -> bool {
    matches!(phase, Some("commentary"))
}

fn push_user_block_lines(
    lines: &mut Vec<Line<'static>>,
    body: &str,
    selected: bool,
    theme: Theme,
    width: u16,
) {
    if push_entrypoint_hint_lines(lines, body, selected, theme, width) {
        return;
    }

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

#[derive(Debug, Clone, Eq, PartialEq)]
struct EntrypointHintRow {
    index: String,
    path: String,
    reason: String,
}

fn push_entrypoint_hint_lines(
    lines: &mut Vec<Line<'static>>,
    body: &str,
    selected: bool,
    theme: Theme,
    width: u16,
) -> bool {
    let Some(rows) = parse_entrypoint_hint_rows(body) else {
        return false;
    };

    let base_style = item_style(theme.muted(), selected, theme);
    let strong_style = item_style(theme.strong(), selected, theme);
    let path_style = item_style(theme.text(), selected, theme).add_modifier(Modifier::BOLD);
    let accent_style = item_style(theme.accent_soft(), selected, theme);
    let row_count = rows.len();

    lines.push(Line::from(vec![
        Span::styled("  ⌖ ", accent_style),
        Span::styled("Likely entry points", strong_style),
        Span::styled(format!(" ({row_count})"), base_style),
    ]));

    let path_width = rows
        .iter()
        .map(|row| row.path.chars().count())
        .max()
        .unwrap_or(0)
        .min(48);
    for row in rows {
        let index = format!("{:>2}. ", row.index);
        let path = truncate_middle(&row.path, path_width.max(12));
        let path_padding = " ".repeat(path_width.saturating_sub(path.chars().count()));
        let reason = truncate_end(&row.reason, reason_width(width, path_width));
        lines.push(Line::from(vec![
            Span::styled("    ", base_style),
            Span::styled(index, accent_style),
            Span::styled(path, path_style),
            Span::styled(path_padding, base_style),
            Span::styled("  ", base_style),
            Span::styled(reason, base_style),
        ]));
    }

    true
}

fn parse_entrypoint_hint_rows(body: &str) -> Option<Vec<EntrypointHintRow>> {
    let mut lines = body.lines();
    let header = lines.next()?.trim();
    if header != "Likely entry points:" {
        return None;
    }

    let rows = lines
        .filter_map(parse_entrypoint_hint_row)
        .collect::<Vec<_>>();
    (!rows.is_empty()).then_some(rows)
}

fn parse_entrypoint_hint_row(line: &str) -> Option<EntrypointHintRow> {
    let (index, rest) = line.trim().split_once(". ")?;
    if index.is_empty() || !index.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let (path, reason) = rest
        .split_once(" - ")
        .unwrap_or((rest, "workspace evidence"));
    let path = path.trim();
    if path.is_empty() {
        return None;
    }

    Some(EntrypointHintRow {
        index: index.to_string(),
        path: path.to_string(),
        reason: reason.trim().to_string(),
    })
}

fn reason_width(width: u16, path_width: usize) -> usize {
    usize::from(width)
        .saturating_sub(4) // row indent
        .saturating_sub(4) // numeric index
        .saturating_sub(path_width)
        .saturating_sub(2) // gap before reason
        .max(16)
}

fn truncate_middle(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    if max_chars <= 3 {
        return "...".chars().take(max_chars).collect();
    }

    let head = (max_chars - 3) / 2;
    let tail = max_chars - 3 - head;
    let start = value.chars().take(head).collect::<String>();
    let end = value
        .chars()
        .rev()
        .take(tail)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{start}...{end}")
}

fn truncate_end(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    if max_chars <= 3 {
        return "...".chars().take(max_chars).collect();
    }
    let mut out = value.chars().take(max_chars - 3).collect::<String>();
    out.push_str("...");
    out
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

fn push_rendered_body_lines(
    lines: &mut Vec<Line<'static>>,
    marker: impl Into<String>,
    body_lines: Vec<Line<'static>>,
    marker_style: Style,
) {
    let marker = marker.into();
    for (line_index, body_line) in body_lines.into_iter().enumerate() {
        let marker = if line_index == 0 || marker.is_empty() {
            marker.clone()
        } else {
            "    ".to_string()
        };
        let mut spans = Vec::with_capacity(body_line.spans.len() + 1);
        spans.push(Span::styled(marker, marker_style));
        spans.extend(body_line.spans);
        let mut line = Line::from(spans);
        line.style = body_line.style;
        line.alignment = body_line.alignment;
        lines.push(line);
    }
}

fn push_reasoning_body_lines(
    lines: &mut Vec<Line<'static>>,
    body: &str,
    marker_style: Style,
    body_style: Style,
) {
    for line in body.split('\n') {
        lines.push(Line::from(vec![
            Span::styled("  ┃ ", marker_style),
            Span::styled(line.to_string(), body_style),
        ]));
    }
}

fn push_reasoning_rendered_lines(
    lines: &mut Vec<Line<'static>>,
    body_lines: Vec<Line<'static>>,
    marker_style: Style,
) {
    for body_line in body_lines {
        let mut spans = Vec::with_capacity(body_line.spans.len() + 1);
        spans.push(Span::styled("  ┃ ", marker_style));
        spans.extend(body_line.spans);
        let mut line = Line::from(spans);
        line.style = body_line.style;
        line.alignment = body_line.alignment;
        lines.push(line);
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

fn is_read_search_tool(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("read")
        || n.contains("search")
        || n.contains("grep")
        || n.contains("glob")
        || n.contains("list")
        || n.contains("view")
        || n.contains("find")
}

fn tool_status_symbol_and_style(
    name: &str,
    status: ToolTimelineStatus,
    animation_frame: u64,
    theme: Theme,
) -> (&'static str, Style) {
    match status {
        ToolTimelineStatus::Running => ("↻", running_tool_marker_style(animation_frame)),
        ToolTimelineStatus::Failed => ("✘", theme.error()),
        ToolTimelineStatus::Completed => {
            if is_read_search_tool(name) {
                ("✔", Style::default().fg(theme.diff_added))
            } else if name.to_lowercase().contains("patch")
                || name.to_lowercase().contains("edit")
                || name.to_lowercase().contains("write")
            {
                ("◆", Style::default().fg(theme.mode_plan))
            } else {
                ("●", Style::default().fg(theme.mode_plan))
            }
        }
    }
}

fn format_tool_arguments(arguments_json: &str) -> String {
    let trimmed = arguments_json.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return String::new();
    }
    let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return trimmed.to_string();
    };
    let serde_json::Value::Object(map) = val else {
        return trimmed.to_string();
    };

    let mut parts = Vec::new();
    let main_keys = ["path", "query", "pattern", "command", "cmd", "url"];
    let mut handled = std::collections::HashSet::new();
    for key in main_keys {
        if let Some(v) = map.get(key) {
            let s = match v {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            if !s.is_empty() {
                parts.push(s);
            }
            handled.insert(key.to_string());
        }
    }

    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for key in keys {
        if handled.contains(key) {
            continue;
        }
        let v = map.get(key).unwrap();
        if v.is_null() {
            continue;
        }
        if let serde_json::Value::String(s) = v
            && s.is_empty()
        {
            continue;
        }
        let val_str = match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        parts.push(format!("{}={}", key, val_str));
    }

    parts.join("  ")
}

fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    let max_width = max_width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current_line = String::new();
        for word in paragraph.split_whitespace() {
            if current_line.is_empty() {
                current_line = word.to_string();
            } else if current_line.chars().count() + 1 + word.chars().count() <= max_width {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                lines.push(current_line);
                current_line = word.to_string();
            }
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }
    }
    lines
}
