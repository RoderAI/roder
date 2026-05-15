use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use crate::diff::compute::{DiffLineKind, Hunk, HunkStatus};
use crate::diff::{DiffViewMode, DiffViewerState};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DiffTheme {
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub added: Color,
    pub removed: Color,
    pub warning: Color,
    pub border: Color,
    pub surface_bg: Color,
}

pub fn diff_viewer_widget(state: &DiffViewerState, theme: DiffTheme) -> Paragraph<'static> {
    Paragraph::new(Text::from(render_lines(state, theme)))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border))
                .style(Style::default().fg(theme.text).bg(theme.surface_bg))
                .title(Span::styled(
                    " diff viewer ",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .style(Style::default().fg(theme.text).bg(theme.surface_bg))
        .wrap(Wrap { trim: false })
}

pub fn render_lines(state: &DiffViewerState, theme: DiffTheme) -> Vec<Line<'static>> {
    let Some(file) = state.current_file() else {
        return vec![Line::from(Span::styled(
            "No diff available",
            Style::default().fg(theme.muted),
        ))];
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                file.path.display().to_string(),
                Style::default().fg(theme.accent),
            ),
            Span::styled(
                format!(
                    "  {}  {}  {}",
                    file.change_type,
                    mode_label(state.mode),
                    if file.supports_partial {
                        "hunk approval"
                    } else {
                        "whole-file fallback"
                    }
                ),
                Style::default().fg(theme.muted),
            ),
        ]),
        Line::from(Span::styled(
            "j/k hunks  y accept  n reject  a accept all  r reject all  s view  Esc close",
            Style::default().fg(theme.muted),
        )),
    ];

    if file.hunks.is_empty() {
        lines.push(Line::from(Span::styled(
            "No textual changes.",
            Style::default().fg(theme.muted),
        )));
        return lines;
    }

    for (index, hunk) in file.hunks.iter().enumerate() {
        lines.push(hunk_header(index, hunk, index == state.hunk_index, theme));
        match state.mode {
            DiffViewMode::Unified => lines.extend(unified_lines(hunk, theme)),
            DiffViewMode::SideBySide => lines.extend(side_by_side_lines(hunk, theme)),
        }
    }
    lines
}

fn hunk_header(index: usize, hunk: &Hunk, selected: bool, theme: DiffTheme) -> Line<'static> {
    let marker = if selected { "> " } else { "  " };
    let status = match hunk.status {
        HunkStatus::Pending => "pending",
        HunkStatus::Accepted => "accepted",
        HunkStatus::Rejected => "rejected",
    };
    let status_color = match hunk.status {
        HunkStatus::Pending => theme.warning,
        HunkStatus::Accepted => theme.added,
        HunkStatus::Rejected => theme.removed,
    };
    Line::from(vec![
        Span::styled(marker.to_string(), Style::default().fg(theme.accent)),
        Span::styled(
            format!(
                "hunk {}  -{}..{} +{}..{}  ",
                index + 1,
                hunk.before_range.start + 1,
                hunk.before_range.end,
                hunk.after_range.start + 1,
                hunk.after_range.end
            ),
            Style::default().fg(theme.muted),
        ),
        Span::styled(status, Style::default().fg(status_color)),
    ])
}

fn unified_lines(hunk: &Hunk, theme: DiffTheme) -> Vec<Line<'static>> {
    hunk.lines
        .iter()
        .map(|line| {
            let (prefix, color) = match line.kind {
                DiffLineKind::Context => (" ", theme.text),
                DiffLineKind::Added => ("+", theme.added),
                DiffLineKind::Removed => ("-", theme.removed),
                DiffLineKind::Binary => ("!", theme.warning),
            };
            Line::from(Span::styled(
                format!("{prefix}{}", line.text.trim_end_matches('\n')),
                Style::default().fg(color),
            ))
        })
        .collect()
}

fn side_by_side_lines(hunk: &Hunk, theme: DiffTheme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for line in &hunk.lines {
        match line.kind {
            DiffLineKind::Context => lines.push(side_by_side_line(&line.text, &line.text, theme)),
            DiffLineKind::Removed => lines.push(side_by_side_line(&line.text, "", theme)),
            DiffLineKind::Added => lines.push(side_by_side_line("", &line.text, theme)),
            DiffLineKind::Binary => lines.push(Line::from(Span::styled(
                line.text.clone(),
                Style::default().fg(theme.warning),
            ))),
        }
    }
    lines
}

fn side_by_side_line(before: &str, after: &str, theme: DiffTheme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<38}", truncate(before.trim_end_matches('\n'), 38)),
            Style::default().fg(if before.is_empty() {
                theme.muted
            } else {
                theme.removed
            }),
        ),
        Span::styled(" | ", Style::default().fg(theme.muted)),
        Span::styled(
            truncate(after.trim_end_matches('\n'), 38),
            Style::default().fg(if after.is_empty() {
                theme.muted
            } else {
                theme.added
            }),
        ),
    ])
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if out.len() < value.len() {
        out.push_str("...");
    }
    out
}

fn mode_label(mode: DiffViewMode) -> &'static str {
    match mode {
        DiffViewMode::Unified => "unified",
        DiffViewMode::SideBySide => "side-by-side",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::compute::{HunkStatus, compute_diff};
    use crate::diff::{DiffViewerState, FileDiff, PendingDiff};

    #[test]
    fn diff_view_unified_render_includes_path_hunks_and_fallback_label() {
        let state = state(false);
        let text = render_text(&state);
        assert!(text.contains("src/lib.rs"));
        assert!(text.contains("whole-file fallback"));
        assert!(text.contains("hunk 1"));
        assert!(text.contains("-two"));
        assert!(text.contains("+TWO"));
    }

    #[test]
    fn diff_view_side_by_side_render_preserves_status_label() {
        let mut state = state(true);
        state.mode = DiffViewMode::SideBySide;
        state.pending.files[0].hunks[0].status = HunkStatus::Accepted;
        let text = render_text(&state);
        assert!(text.contains("side-by-side"));
        assert!(text.contains("accepted"));
        assert!(text.contains(" | "));
    }

    fn render_text(state: &DiffViewerState) -> String {
        render_lines(state, theme())
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn state(supports_partial: bool) -> DiffViewerState {
        DiffViewerState::new(PendingDiff {
            call_id: "call-a".to_string(),
            tool: "edit".to_string(),
            files: vec![FileDiff {
                path: "src/lib.rs".into(),
                change_type: "modify".to_string(),
                before: Some("one\ntwo\nthree\n".to_string()),
                after: "one\nTWO\nthree\n".to_string(),
                supports_partial,
                hunks: compute_diff(Some("one\ntwo\nthree\n"), "one\nTWO\nthree\n"),
            }],
        })
    }

    fn theme() -> DiffTheme {
        DiffTheme {
            text: Color::Reset,
            muted: Color::Indexed(244),
            accent: Color::Indexed(212),
            added: Color::Indexed(40),
            removed: Color::Indexed(160),
            warning: Color::Indexed(214),
            border: Color::Indexed(244),
            surface_bg: Color::Reset,
        }
    }
}
