use std::collections::{BTreeMap, BTreeSet};

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use roder_api::tui_status::{StatusCell, StatusContext, StatusSegment, StatusStyle};

pub use roder_api::tui_status::built_in_status_segments;

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use roder_api::policy_mode::PolicyMode;
    use roder_api::tui_status::{
        GitSnapshot, McpServerStatus, StatusCell, ThreadSummary, ThreadUsage,
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
        assert!(text.contains(&"profile:gpt-test"));
        assert!(text.contains(&"thread:12345678"));
        assert!(text.contains(&"line:main"));
        assert!(text.contains(&"tok:42"));
        assert!(text.contains(&"mcp:1"));
    }

    #[test]
    fn model_switch_status_marks_profile_segment_warning() {
        let mut ctx = test_context();
        ctx.model_profile = Some("claude-haiku-4-5-20251001");
        ctx.model_switch_summary = Some("Model switch summary: previous profile mock/gpt-5.5.");

        let cells = status_cells(
            &built_in_status_segments(),
            &ctx,
            120,
            &StatusLineConfig::default(),
        );
        let profile = cells
            .iter()
            .find(|cell| cell.text.starts_with("profile:"))
            .unwrap();
        assert_eq!(profile.text, "profile:claude-haiku-4-5-20251001");
        assert_eq!(profile.style, StatusStyle::Warning);
        assert_eq!(
            profile.tooltip,
            ctx.model_switch_summary.map(str::to_string)
        );
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
        static THREAD: std::sync::OnceLock<ThreadSummary> = std::sync::OnceLock::new();
        static USAGE: std::sync::OnceLock<ThreadUsage> = std::sync::OnceLock::new();
        static GIT: std::sync::OnceLock<GitSnapshot> = std::sync::OnceLock::new();
        static MCP: std::sync::OnceLock<Vec<McpServerStatus>> = std::sync::OnceLock::new();

        StatusContext {
            thread: THREAD.get_or_init(|| ThreadSummary {
                thread_id: "1234567890abcdef".to_string(),
                title: Some("Test".to_string()),
            }),
            policy_mode: PolicyMode::Plan,
            model: Some("gpt-test"),
            model_profile: Some("gpt-test"),
            model_switch_summary: None,
            usage: Some(USAGE.get_or_init(|| ThreadUsage {
                input_tokens: 40,
                output_tokens: 2,
                total_cost_usd: None,
            })),
            git: Some(GIT.get_or_init(|| GitSnapshot {
                branch: Some("main".to_string()),
            })),
            vcs: None,
            mcp: MCP.get_or_init(|| {
                vec![McpServerStatus {
                    id: "local".to_string(),
                    state: "ready".to_string(),
                }]
            }),
            runner: None,
        }
    }
}
