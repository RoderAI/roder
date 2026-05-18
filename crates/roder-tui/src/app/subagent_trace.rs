use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use roder_api::trace::{
    SubagentTraceDelta, SubagentTraceItem, SubagentTraceStatus, SubagentTraceSummary,
};

use super::Theme;

#[derive(Debug, Clone)]
pub(super) struct SubagentTraceRow {
    summary: SubagentTraceSummary,
    items: Vec<String>,
}

impl PartialEq for SubagentTraceRow {
    fn eq(&self, other: &Self) -> bool {
        self.summary.trace_id == other.summary.trace_id
            && self.summary.status == other.summary.status
            && self.items == other.items
    }
}

impl Eq for SubagentTraceRow {}

impl SubagentTraceRow {
    pub(super) fn new(summary: SubagentTraceSummary) -> Self {
        Self {
            summary,
            items: Vec::new(),
        }
    }

    pub(super) fn trace_id(&self) -> &str {
        &self.summary.trace_id
    }

    pub(super) fn has_items(&self) -> bool {
        !self.items.is_empty()
    }

    pub(super) fn update_summary(&mut self, summary: SubagentTraceSummary) {
        self.summary = summary;
    }

    pub(super) fn update_status(&mut self, status: SubagentTraceStatus, detail: Option<String>) {
        self.summary.status = status;
        if let Some(detail) = detail.filter(|detail| !detail.trim().is_empty()) {
            self.summary.latest_activity = Some(detail);
        }
    }

    pub(super) fn push_delta(&mut self, delta: SubagentTraceDelta) {
        self.items.push(trace_item_label(delta.item));
        if let Some(item) = self.items.last() {
            self.summary.latest_activity = Some(item.clone());
        }
    }

    pub(super) fn render(
        &self,
        selected: bool,
        expanded: bool,
        theme: Theme,
        animation_frame: u64,
        lines: &mut Vec<Line<'static>>,
    ) {
        let marker_style = status_style(&self.summary.status, theme, animation_frame);
        let affordance = if self.has_items() {
            if expanded { "▾" } else { "▸" }
        } else {
            " "
        };
        let status = status_label(&self.summary.status);
        let destination = self
            .summary
            .destination
            .as_ref()
            .map(|destination| destination.label.as_str())
            .unwrap_or("workspace");
        let latest = self
            .summary
            .latest_activity
            .as_deref()
            .filter(|activity| !activity.trim().is_empty())
            .map(|activity| format!(" - {activity}"))
            .unwrap_or_default();
        let elapsed = format_duration(std::time::Duration::from_millis(self.summary.elapsed_ms));
        lines.push(Line::from(vec![
            Span::styled("  ◇ ", marker_style),
            Span::styled(affordance.to_string(), theme.subtle()),
            Span::styled(
                format!(
                    " {}: {}  {status}  {elapsed}  {destination}{latest}",
                    self.summary.role, self.summary.title
                ),
                item_style(
                    status_body_style(&self.summary.status, theme),
                    selected,
                    theme,
                ),
            ),
        ]));

        if expanded {
            for item in self.items.iter().take(24) {
                lines.push(Line::from(vec![
                    Span::styled("    ", theme.subtle()),
                    Span::styled(item.clone(), theme.muted()),
                ]));
            }
            if self.items.len() > 24 {
                lines.push(Line::from(Span::styled(
                    "    trace preview truncated in timeline",
                    theme.muted().add_modifier(Modifier::ITALIC),
                )));
            }
        }
    }
}

fn trace_item_label(item: SubagentTraceItem) -> String {
    match item {
        SubagentTraceItem::Message { role, content } => {
            format!("{role}: {}", single_line(content.text))
        }
        SubagentTraceItem::Reasoning { content } => {
            format!("thinking: {}", single_line(content.text))
        }
        SubagentTraceItem::ToolCall { tool_name, .. } => format!("tool: {tool_name}"),
        SubagentTraceItem::ToolResult {
            is_error, output, ..
        } => {
            let prefix = if is_error {
                "tool error"
            } else {
                "tool result"
            };
            format!("{prefix}: {}", single_line(output.text))
        }
        SubagentTraceItem::Status { status, detail } => detail
            .map(|detail| format!("{}: {detail}", status_label(&status)))
            .unwrap_or_else(|| status_label(&status).to_string()),
    }
}

fn single_line(text: String) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() > 96 {
        format!("{}...", line.chars().take(96).collect::<String>())
    } else {
        line.to_string()
    }
}

fn status_label(status: &SubagentTraceStatus) -> &'static str {
    match status {
        SubagentTraceStatus::Queued => "queued",
        SubagentTraceStatus::Running => "running",
        SubagentTraceStatus::WaitingForApproval => "approval",
        SubagentTraceStatus::Completed => "done",
        SubagentTraceStatus::Failed => "failed",
        SubagentTraceStatus::Cancelled => "cancelled",
    }
}

fn status_style(status: &SubagentTraceStatus, theme: Theme, animation_frame: u64) -> Style {
    match status {
        SubagentTraceStatus::Running => {
            let colors = [33, 39, 45, 81, 45, 39];
            Style::default()
                .fg(ratatui::style::Color::Indexed(
                    colors[(animation_frame as usize) % colors.len()],
                ))
                .add_modifier(Modifier::BOLD)
        }
        SubagentTraceStatus::Failed => theme.error(),
        SubagentTraceStatus::Completed => theme.tool(),
        _ => theme.subtle(),
    }
}

fn status_body_style(status: &SubagentTraceStatus, theme: Theme) -> Style {
    match status {
        SubagentTraceStatus::Failed => theme.error(),
        SubagentTraceStatus::Running | SubagentTraceStatus::WaitingForApproval => theme.text(),
        _ => theme.muted(),
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
    format!("{minutes} min {seconds} sec")
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
