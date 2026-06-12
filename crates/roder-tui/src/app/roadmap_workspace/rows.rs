use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::ListItem,
};
use roder_roadmap::{DocumentSummary, ThreadAttachment};
use time::OffsetDateTime;

use super::super::{Theme, short_id};
use crate::roadmap::RoadmapModeState;

pub(super) struct TaskStatusView {
    pub glyph: &'static str,
    pub label: &'static str,
    kind: TaskStatusKind,
}

enum TaskStatusKind {
    Done,
    Assigned,
    Ready,
    Pending,
}

impl TaskStatusView {
    pub(super) fn style(&self, theme: Theme) -> Style {
        match self.kind {
            TaskStatusKind::Done => theme.accent_soft(),
            TaskStatusKind::Assigned => theme.running(),
            TaskStatusKind::Ready => theme.accent(),
            TaskStatusKind::Pending => theme.subtle(),
        }
    }
}

pub(super) fn task_status(
    state: &RoadmapModeState,
    task_id: &str,
    checked: bool,
    focused: bool,
) -> TaskStatusView {
    if checked {
        TaskStatusView {
            glyph: "✓",
            label: "done",
            kind: TaskStatusKind::Done,
        }
    } else if workers_for_task(state, task_id) > 0 {
        TaskStatusView {
            glyph: "●",
            label: "assigned",
            kind: TaskStatusKind::Assigned,
        }
    } else if focused {
        TaskStatusView {
            glyph: "▸",
            label: "ready",
            kind: TaskStatusKind::Ready,
        }
    } else {
        TaskStatusView {
            glyph: "·",
            label: "pending",
            kind: TaskStatusKind::Pending,
        }
    }
}

pub(super) fn workers_for_task(state: &RoadmapModeState, task_id: &str) -> usize {
    state
        .attached_threads
        .iter()
        .filter(|thread| thread.task_id.as_deref() == Some(task_id))
        .count()
}

pub(super) fn worker_row(
    thread: &ThreadAttachment,
    selected: bool,
    last: bool,
    inner_w: usize,
    theme: Theme,
) -> ListItem<'static> {
    let connector = if last { "╰─" } else { "├─" };
    let status = thread.status.clone().unwrap_or_default();
    let (glyph, glyph_style) = match status.as_str() {
        "done" | "completed" => ("✓", theme.accent_soft()),
        "failed" | "error" => ("!", theme.error()),
        _ => ("●", theme.running()),
    };
    let id = short_id(&thread.thread_id).to_string();
    let task = thread
        .task_id
        .as_deref()
        .map(|task| task.trim_start_matches("task-").to_string())
        .unwrap_or_else(|| "unassigned".to_string());
    let elapsed = elapsed_since(thread.updated_at);
    let task_width = inner_w
        .saturating_sub(id.chars().count() + elapsed.chars().count() + 8)
        .max(4);
    ListItem::new(Line::from(vec![
        Span::styled(connector.to_string(), theme.border()),
        Span::styled(format!(" {glyph} "), glyph_style),
        Span::styled(
            id,
            if selected {
                theme.selected()
            } else {
                theme.text()
            },
        ),
        Span::styled(format!(" {}", clip(&task, task_width)), theme.muted()),
        Span::styled(format!(" {elapsed}"), theme.subtle()),
    ]))
}

pub(super) struct ListWindow {
    pub start: usize,
    pub end: usize,
    pub clipped_above: usize,
    pub clipped_below: usize,
}

pub(super) fn list_window(len: usize, selected: usize, height: usize) -> ListWindow {
    if height == 0 || len <= height {
        return ListWindow {
            start: 0,
            end: len,
            clipped_above: 0,
            clipped_below: 0,
        };
    }
    let mut start = selected.saturating_sub(height / 2);
    if start + height > len {
        start = len - height;
    }
    let end = start + height;
    let mut window = ListWindow {
        start,
        end,
        clipped_above: start,
        clipped_below: len - end,
    };
    // Reserve a row for each "more" indicator so the pane never overflows,
    // but only when the pane is tall enough to keep the selection visible.
    if height >= 3 {
        if window.clipped_above > 0 {
            window.start += 1;
            window.clipped_above = window.start;
        }
        if window.clipped_below > 0 {
            window.end -= 1;
            window.clipped_below = len - window.end;
        }
    }
    window
}

pub(super) fn overflow_row(count: usize, direction: &str, theme: Theme) -> ListItem<'static> {
    let arrow = if direction == "above" { "↑" } else { "↓" };
    ListItem::new(Line::from(Span::styled(
        format!("  {arrow} {count} more"),
        theme.subtle(),
    )))
}

pub(super) fn clip(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

pub(super) fn strip_plan_suffix(title: &str) -> &str {
    title
        .trim()
        .strip_suffix(" Implementation Plan")
        .unwrap_or_else(|| title.trim())
}

pub(super) fn plan_display_title(document: &DocumentSummary) -> String {
    let title = strip_plan_suffix(&document.title);
    if !title.is_empty() {
        return title.to_string();
    }
    document
        .path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| "untitled".to_string())
}

pub(super) fn progress_bar(filled: usize, total: usize, width: usize) -> String {
    if total == 0 || width == 0 {
        return String::new();
    }
    let cells = (filled * width).div_ceil(total).min(width);
    let mut bar = String::with_capacity(width * 3);
    for _ in 0..cells {
        bar.push('█');
    }
    for _ in cells..width {
        bar.push('░');
    }
    bar
}

pub(super) fn progress_stats(checked: usize, total: usize) -> String {
    if total == 0 {
        return String::new();
    }
    format!("{checked}/{total} · {}%", checked * 100 / total)
}

pub(super) fn elapsed_since(when: OffsetDateTime) -> String {
    let seconds = (OffsetDateTime::now_utc() - when).whole_seconds().max(0) as u64;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

pub(super) fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_window_keeps_selection_visible_and_reports_clipping() {
        let window = list_window(100, 50, 10);
        assert!(window.start <= 50 && 50 < window.end);
        assert!(window.clipped_above > 0);
        assert!(window.clipped_below > 0);
        assert!(window.end - window.start <= 10);

        let top = list_window(100, 0, 10);
        assert_eq!(top.start, 0);
        assert_eq!(top.clipped_above, 0);
        assert!(top.clipped_below > 0);

        let bottom = list_window(100, 99, 10);
        assert_eq!(bottom.end, 100);
        assert_eq!(bottom.clipped_below, 0);
        assert!(bottom.clipped_above > 0);

        let small = list_window(3, 1, 10);
        assert_eq!((small.start, small.end), (0, 3));
        assert_eq!(small.clipped_above + small.clipped_below, 0);
    }

    #[test]
    fn progress_bar_renders_proportional_fill() {
        assert_eq!(progress_bar(0, 10, 10), "░░░░░░░░░░");
        assert_eq!(progress_bar(10, 10, 10), "██████████");
        assert_eq!(progress_bar(5, 10, 10), "█████░░░░░");
        assert_eq!(progress_bar(1, 34, 18), "█░░░░░░░░░░░░░░░░░");
        assert_eq!(progress_bar(0, 0, 10), "");
    }

    #[test]
    fn progress_stats_formats_percentages() {
        assert_eq!(progress_stats(12, 34), "12/34 · 35%");
        assert_eq!(progress_stats(0, 0), "");
    }

    #[test]
    fn clip_fits_exact_width_with_ellipsis() {
        assert_eq!(clip("abcdef", 6), "abcdef");
        assert_eq!(clip("abcdefg", 6), "abcde…");
        assert_eq!(clip("abc", 0), "");
    }

    #[test]
    fn plan_titles_strip_suffix_and_fall_back_to_file_stem() {
        assert_eq!(
            strip_plan_suffix("Goal Mode Implementation Plan"),
            "Goal Mode"
        );
        let document = DocumentSummary {
            id: "x".to_string(),
            path: std::path::PathBuf::from("roadmap/93-loop.md"),
            title: String::new(),
            checked_tasks: 0,
            unchecked_tasks: 0,
        };
        assert_eq!(plan_display_title(&document), "93-loop");
    }
}
