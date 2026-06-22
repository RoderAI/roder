use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use crate::diff::compute::{DiffLineKind, Hunk, HunkStatus};
use crate::diff::{DiffViewMode, DiffViewerState};
use roder_tui_syntax::{SyntaxTheme, highlight_code, language_for_path, padded_highlighted_code};

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
    pub border_type: BorderType,
    pub borders_visible: bool,
}

pub fn diff_viewer_widget(state: &DiffViewerState, theme: DiffTheme) -> Paragraph<'static> {
    let borders = if theme.borders_visible {
        Borders::ALL
    } else {
        Borders::NONE
    };
    Paragraph::new(Text::from(render_lines(state, theme)))
        .block(
            Block::default()
                .borders(borders)
                .border_type(theme.border_type)
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
                    "  file {}/{}  {}  {}  {}",
                    state.file_index + 1,
                    state.file_count(),
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
            "J/K files  j/k hunks  y accept  n reject  a accept all  r reject all  s view  Esc close",
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
        let language = language_for_path(&file.path);
        match state.mode {
            DiffViewMode::Unified => lines.extend(unified_lines(hunk, theme, language)),
            DiffViewMode::SideBySide => lines.extend(side_by_side_lines(hunk, theme, language)),
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

fn unified_lines(
    hunk: &Hunk,
    theme: DiffTheme,
    language: Option<roder_tui_syntax::SyntaxLanguage>,
) -> Vec<Line<'static>> {
    hunk.lines
        .iter()
        .map(|line| {
            let (prefix, color) = match line.kind {
                DiffLineKind::Context => (" ", theme.text),
                DiffLineKind::Added => ("+", theme.added),
                DiffLineKind::Removed => ("-", theme.removed),
                DiffLineKind::Binary => ("!", theme.warning),
            };
            let mut spans = vec![Span::styled(prefix.to_string(), Style::default().fg(color))];
            spans.extend(highlight_code(
                line.text.trim_end_matches('\n'),
                language,
                syntax_theme(color, theme),
            ));
            Line::from(spans)
        })
        .collect()
}

fn side_by_side_lines(
    hunk: &Hunk,
    theme: DiffTheme,
    language: Option<roder_tui_syntax::SyntaxLanguage>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for line in &hunk.lines {
        match line.kind {
            DiffLineKind::Context => {
                lines.push(side_by_side_line(&line.text, &line.text, theme, language))
            }
            DiffLineKind::Removed => lines.push(side_by_side_line(&line.text, "", theme, language)),
            DiffLineKind::Added => lines.push(side_by_side_line("", &line.text, theme, language)),
            DiffLineKind::Binary => lines.push(Line::from(Span::styled(
                line.text.clone(),
                Style::default().fg(theme.warning),
            ))),
        }
    }
    lines
}

fn side_by_side_line(
    before: &str,
    after: &str,
    theme: DiffTheme,
    language: Option<roder_tui_syntax::SyntaxLanguage>,
) -> Line<'static> {
    let before_text = truncate(before.trim_end_matches('\n'), 38);
    let after_text = truncate(after.trim_end_matches('\n'), 38);
    let before_color = if before.is_empty() {
        theme.muted
    } else {
        theme.removed
    };
    let after_color = if after.is_empty() {
        theme.muted
    } else {
        theme.added
    };
    let mut spans = Vec::new();
    spans.extend(padded_highlighted_code(
        &before_text,
        38,
        language,
        syntax_theme(before_color, theme),
    ));
    spans.push(Span::styled(" | ", Style::default().fg(theme.muted)));
    spans.extend(highlight_code(
        &after_text,
        language,
        syntax_theme(after_color, theme),
    ));
    Line::from(spans)
}

fn syntax_theme(base: Color, theme: DiffTheme) -> SyntaxTheme {
    SyntaxTheme {
        base,
        keyword: theme.accent,
        string: theme.warning,
        number: theme.warning,
        comment: theme.muted,
        ty: theme.accent,
        function: theme.text,
        mac: theme.warning,
        bg: None,
    }
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
        assert!(text.contains("file 1/1"));
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

    #[test]
    fn diff_view_multi_file_render_shows_position_and_selected_file() {
        let mut state = state(true);
        state.pending.files.push(FileDiff {
            path: "src/other.rs".into(),
            change_type: "modify".to_string(),
            before: Some("alpha\nbeta\n".to_string()),
            after: "alpha\nBETA\n".to_string(),
            supports_partial: true,
            hunks: compute_diff(Some("alpha\nbeta\n"), "alpha\nBETA\n"),
        });
        state.next_file();

        let text = render_text(&state);
        assert!(text.contains("src/other.rs"));
        assert!(text.contains("file 2/2"));
        assert!(!text.contains("src/lib.rs  file"));
    }

    #[test]
    fn diff_view_highlights_syntax_inside_diff_lines() {
        let state = DiffViewerState::new(PendingDiff {
            call_id: "call-a".to_string(),
            tool: "edit".to_string(),
            files: vec![FileDiff {
                path: "src/lib.rs".into(),
                change_type: "modify".to_string(),
                before: Some("let value = old();\n".to_string()),
                after: "let value = format!(\"new\");\n".to_string(),
                supports_partial: true,
                hunks: compute_diff(
                    Some("let value = old();\n"),
                    "let value = format!(\"new\");\n",
                ),
            }],
        });
        let theme = theme();
        let lines = render_lines(&state, theme);

        assert!(lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|span| span.content.as_ref() == "let" && span.style.fg == Some(theme.accent))
        }));
        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.as_ref() == r#""new""# && span.style.fg == Some(theme.warning)
            })
        }));
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
            border_type: BorderType::Rounded,
            borders_visible: true,
        }
    }
}
