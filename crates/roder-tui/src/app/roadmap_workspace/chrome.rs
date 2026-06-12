use ratatui::{
    text::{Line, Span, Text},
    widgets::Paragraph,
};

use super::super::Theme;
use super::RoadmapWorkspaceMeta;
use super::rows::{clip, progress_bar, progress_stats, strip_plan_suffix};
use crate::roadmap::RoadmapModeState;

pub(super) fn header(
    state: &RoadmapModeState,
    theme: Theme,
    meta: &RoadmapWorkspaceMeta,
    width: u16,
) -> Paragraph<'static> {
    let (checked, total) = state
        .selected_document
        .as_ref()
        .map(|document| {
            let total = document.tasks.len();
            let checked = document.tasks.iter().filter(|task| task.checked).count();
            (checked, total)
        })
        .unwrap_or((0, 0));
    let title = state
        .selected_document
        .as_ref()
        .map(|document| strip_plan_suffix(&document.title).to_string())
        .unwrap_or_else(|| "select a roadmap".to_string());
    let lead = if meta.active_turn && !meta.spinner.is_empty() {
        meta.spinner.clone()
    } else {
        "◆".to_string()
    };

    let bar_width = 18usize;
    let stats = progress_stats(checked, total);
    let reserved = 12 + bar_width + stats.chars().count();
    let title_width = usize::from(width).saturating_sub(reserved).max(8);
    let mut first = vec![
        Span::styled(format!(" {lead} "), theme.accent()),
        Span::styled("Roadmap ", theme.accent()),
        Span::styled(clip(&title, title_width), theme.strong()),
        Span::styled("  ", theme.text()),
    ];
    if total > 0 {
        first.push(Span::styled(
            progress_bar(checked, total, bar_width),
            theme.accent_soft(),
        ));
        first.push(Span::styled(format!(" {stats}"), theme.muted()));
    }

    let workers = state.attached_threads.len();
    let worker_label = format!(
        "{workers} {}",
        if workers == 1 { "worker" } else { "workers" }
    );
    let mut second = vec![
        Span::styled("   ", theme.text()),
        Span::styled(meta.status.clone(), theme.muted()),
        Span::styled(" · ", theme.subtle()),
        Span::styled(meta.model.clone(), theme.muted()),
        Span::styled(" · ", theme.subtle()),
        validation_chip(state, theme),
        Span::styled(" · ", theme.subtle()),
        Span::styled(
            worker_label,
            if workers > 0 {
                theme.running()
            } else {
                theme.muted()
            },
        ),
    ];
    // The plan path is the least important detail; show it only when it fits.
    if let Some(plan) = state.selected_plan.as_deref() {
        let used: usize = second.iter().map(|span| span.content.chars().count()).sum();
        if used + plan.chars().count() + 3 <= usize::from(width) {
            second.push(Span::styled(" · ", theme.subtle()));
            second.push(Span::styled(plan.to_string(), theme.muted()));
        }
    }

    Paragraph::new(Text::from(vec![Line::from(first), Line::from(second)]))
}

fn validation_chip(state: &RoadmapModeState, theme: Theme) -> Span<'static> {
    let issues = state.validation_diagnostics.len();
    if issues == 0 {
        Span::styled("✓ valid", theme.accent_soft())
    } else {
        Span::styled(format!("! {issues} issues"), theme.error())
    }
}

pub(super) fn footer(theme: Theme, width: u16) -> Paragraph<'static> {
    let mut spans = Vec::new();
    let mut used = 1usize;
    spans.push(Span::styled(" ", theme.text()));
    for (key, label) in [
        ("tab", "panes"),
        ("j/k", "move"),
        ("s", "spawn"),
        ("e", "exec"),
        ("v", "validate"),
        ("enter", "open"),
        ("t", "worker"),
        ("esc", "leave"),
    ] {
        let cost = key.chars().count() + label.chars().count() + 5;
        if used + cost > usize::from(width) {
            break;
        }
        used += cost;
        spans.push(Span::styled(format!(" {key} "), theme.dialog_key()));
        spans.push(Span::styled(format!(" {label}  "), theme.muted()));
    }
    Paragraph::new(Line::from(spans))
}
