use std::collections::{BTreeMap, BTreeSet};

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use roder_api::policy_mode::PolicyMode;
use roder_api::tui_status::{StatusCell, StatusContext, StatusSegment, StatusStyle};

#[derive(Debug, Clone, Default)]
pub struct StatusLineConfig {
    pub disabled_segments: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusLineTheme {
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub warning: Color,
    pub error: Color,
    pub separator: Color,
}

pub fn built_in_status_segments() -> Vec<StatusSegment> {
    vec![
        StatusSegment::new("mode", 100, 8, |ctx| StatusCell {
            text: format!("mode:{}", policy_mode_label(ctx.policy_mode)),
            style: StatusStyle::Accent,
            tooltip: Some("Active policy mode".to_string()),
        }),
        StatusSegment::new("model", 90, 8, |ctx| StatusCell {
            text: ctx
                .model
                .map(|model| format!("model:{model}"))
                .unwrap_or_else(|| "model:-".to_string()),
            style: StatusStyle::Default,
            tooltip: Some("Active model".to_string()),
        }),
        StatusSegment::new("session", 80, 8, |ctx| StatusCell {
            text: format!("session:{}", short_id(&ctx.session.thread_id)),
            style: StatusStyle::Muted,
            tooltip: ctx.session.title.clone(),
        }),
        StatusSegment::new("branch", 70, 8, |ctx| StatusCell {
            text: ctx
                .git
                .and_then(|git| git.branch.as_deref())
                .map(|branch| format!("branch:{branch}"))
                .unwrap_or_else(|| "branch:-".to_string()),
            style: StatusStyle::Muted,
            tooltip: Some("Best-effort git branch".to_string()),
        }),
        StatusSegment::new("usage", 60, 8, |ctx| StatusCell {
            text: ctx
                .usage
                .map(|usage| format!("tok:{}", usage.input_tokens + usage.output_tokens))
                .unwrap_or_else(|| "tok:-".to_string()),
            style: StatusStyle::Muted,
            tooltip: Some("Session token usage".to_string()),
        }),
        StatusSegment::new("mcp", 50, 6, |ctx| StatusCell {
            text: format!("mcp:{}", ctx.mcp.len()),
            style: StatusStyle::Muted,
            tooltip: Some("Configured MCP servers".to_string()),
        }),
    ]
}

pub fn render_status_line(
    segments: &[StatusSegment],
    ctx: &StatusContext<'_>,
    width: u16,
    config: &StatusLineConfig,
    theme: StatusLineTheme,
) -> Paragraph<'static> {
    let cells = status_cells(segments, ctx, width, config);
    let mut spans = Vec::new();
    for (index, cell) in cells.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" | ", Style::default().fg(theme.separator)));
        }
        let style = style_for_cell(&cell, theme);
        spans.push(Span::styled(cell.text, style));
    }
    Paragraph::new(Line::from(spans))
}

pub fn status_cells(
    segments: &[StatusSegment],
    ctx: &StatusContext<'_>,
    width: u16,
    config: &StatusLineConfig,
) -> Vec<StatusCell> {
    let mut by_id = BTreeMap::new();
    for segment in segments {
        if !config.disabled_segments.contains(&segment.id) {
            by_id.insert(segment.id.clone(), segment.clone());
        }
    }
    let mut segments = by_id.into_values().collect::<Vec<_>>();
    segments.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));

    let mut used = 0usize;
    let mut cells = Vec::new();
    let max = usize::from(width);
    for segment in segments {
        let mut cell = (segment.render)(ctx);
        if cell.text.trim().is_empty() {
            continue;
        }
        let separator = if cells.is_empty() { 0 } else { 3 };
        let min_width = usize::from(segment.min_width);
        if used + separator + min_width > max {
            continue;
        }
        let remaining = max.saturating_sub(used + separator);
        cell.text = truncate_to_width(&cell.text, remaining);
        used += separator + cell.text.chars().count();
        cells.push(cell);
    }
    cells
}

fn style_for_cell(cell: &StatusCell, theme: StatusLineTheme) -> Style {
    match cell.style {
        StatusStyle::Default => Style::default().fg(theme.text),
        StatusStyle::Muted => Style::default().fg(theme.muted),
        StatusStyle::Accent => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
        StatusStyle::Warning => Style::default().fg(theme.warning),
        StatusStyle::Error => Style::default().fg(theme.error),
    }
}

fn truncate_to_width(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let mut out = text.chars().take(width - 3).collect::<String>();
    out.push_str("...");
    out
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn policy_mode_label(mode: PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Default => "default",
        PolicyMode::AcceptEdits => "accept_edits",
        PolicyMode::Plan => "plan",
        PolicyMode::Bypass => "bypass",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use roder_api::tui_status::{
        GitSnapshot, McpServerStatus, SessionSummary, SessionUsage, StatusCell,
    };

    use super::*;

    #[test]
    fn status_line_orders_by_priority_and_truncates() {
        let ctx = test_context();
        let segments = vec![
            segment("low", 10, "low-segment"),
            segment("high", 100, "high-segment"),
            segment("mid", 50, "mid-segment"),
        ];

        let cells = status_cells(&segments, &ctx, 24, &StatusLineConfig::default());
        assert_eq!(
            cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<Vec<_>>(),
            ["high-segment", "mid-se..."]
        );
    }

    #[test]
    fn status_line_can_disable_segment_by_id() {
        let ctx = test_context();
        let mut disabled_segments = BTreeSet::new();
        disabled_segments.insert("mode".to_string());
        let cells = status_cells(
            &built_in_status_segments(),
            &ctx,
            80,
            &StatusLineConfig { disabled_segments },
        );
        assert!(!cells.iter().any(|cell| cell.text.starts_with("mode:")));
    }

    #[test]
    fn contributor_segment_replaces_default_with_same_id() {
        let ctx = test_context();
        let mut segments = built_in_status_segments();
        segments.push(segment("mode", 200, "custom-mode"));

        let cells = status_cells(&segments, &ctx, 80, &StatusLineConfig::default());
        assert!(cells.iter().any(|cell| cell.text == "custom-mode"));
        assert!(!cells.iter().any(|cell| cell.text.starts_with("mode:")));
    }

    #[test]
    fn builtins_render_expected_status_values() {
        let ctx = test_context();
        let cells = status_cells(
            &built_in_status_segments(),
            &ctx,
            120,
            &StatusLineConfig::default(),
        );
        let text = cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<Vec<_>>();
        assert!(text.contains(&"mode:plan"));
        assert!(text.contains(&"model:gpt-test"));
        assert!(text.contains(&"session:12345678"));
        assert!(text.contains(&"branch:main"));
        assert!(text.contains(&"tok:42"));
        assert!(text.contains(&"mcp:1"));
    }

    fn segment(id: &str, priority: i32, text: &str) -> StatusSegment {
        let text = text.to_string();
        StatusSegment::new(id, priority, 3, move |_| StatusCell {
            text: text.clone(),
            style: StatusStyle::Default,
            tooltip: None,
        })
    }

    fn test_context() -> StatusContext<'static> {
        static SESSION: std::sync::OnceLock<SessionSummary> = std::sync::OnceLock::new();
        static USAGE: std::sync::OnceLock<SessionUsage> = std::sync::OnceLock::new();
        static GIT: std::sync::OnceLock<GitSnapshot> = std::sync::OnceLock::new();
        static MCP: std::sync::OnceLock<Vec<McpServerStatus>> = std::sync::OnceLock::new();

        StatusContext {
            session: SESSION.get_or_init(|| SessionSummary {
                thread_id: "1234567890abcdef".to_string(),
                title: Some("Test".to_string()),
            }),
            policy_mode: PolicyMode::Plan,
            model: Some("gpt-test"),
            usage: Some(USAGE.get_or_init(|| SessionUsage {
                input_tokens: 40,
                output_tokens: 2,
                total_cost_usd: None,
            })),
            git: Some(GIT.get_or_init(|| GitSnapshot {
                branch: Some("main".to_string()),
            })),
            mcp: MCP.get_or_init(|| {
                vec![McpServerStatus {
                    id: "local".to_string(),
                    state: "ready".to_string(),
                }]
            }),
        }
    }
}
