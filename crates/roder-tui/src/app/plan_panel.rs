use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};

use super::{Theme, plural_s};

const MAX_VISIBLE_PLAN_ITEMS: usize = 8;

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(super) struct PlanPanelState {
    items: Vec<PlanItem>,
    visible: bool,
}

impl PlanPanelState {
    pub(super) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(super) fn is_visible(&self) -> bool {
        self.visible && !self.items.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    pub(super) fn completed_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == PlanStatus::Completed)
            .count()
    }

    pub(super) fn toggle(&mut self) {
        if !self.items.is_empty() {
            self.visible = !self.visible;
        }
    }

    pub(super) fn replace_from_update_plan_output(&mut self, output: &str) {
        self.items = parse_plan_items(output);
        self.visible = !self.items.is_empty();
    }

    pub(super) fn replace_from_task_ledger_output(&mut self, output: &str) {
        self.items = parse_plan_items(output);
        self.visible = !self.items.is_empty();
    }

    fn visible_items(&self) -> impl Iterator<Item = &PlanItem> {
        self.items.iter().take(MAX_VISIBLE_PLAN_ITEMS)
    }

    fn hidden_count(&self) -> usize {
        self.items.len().saturating_sub(MAX_VISIBLE_PLAN_ITEMS)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct PlanItem {
    step: String,
    status: PlanStatus,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PlanStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
}

pub(super) fn plan_panel_height(plan: &PlanPanelState) -> u16 {
    if !plan.is_visible() {
        return 0;
    }
    let visible = plan.len().min(MAX_VISIBLE_PLAN_ITEMS);
    let hidden_row = usize::from(plan.hidden_count() > 0);
    (visible + hidden_row) as u16
}

pub(super) fn render_plan_panel(plan: &PlanPanelState, theme: Theme) -> Paragraph<'static> {
    let mut lines = plan
        .visible_items()
        .map(|item| {
            let (marker, style) = status_marker(item.status, theme);
            Line::from(vec![
                Span::styled("  ", theme.subtle()),
                Span::styled(marker, style),
                Span::styled(truncate_middle(&item.step, 140), theme.muted()),
            ])
        })
        .collect::<Vec<_>>();

    let hidden = plan.hidden_count();
    if hidden > 0 {
        lines.push(Line::from(vec![
            Span::styled("  ", theme.subtle()),
            Span::styled("› ", theme.subtle()),
            Span::styled(
                format!("{hidden} more todo{}", plural_s(hidden)),
                theme.muted().add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    Paragraph::new(Text::from(lines))
        .style(theme.text())
        .wrap(Wrap { trim: false })
}

pub(super) fn render_plan_counter(plan: &PlanPanelState, theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        Span::styled("│ ", theme.muted()),
        Span::styled(
            plan.len().to_string(),
            plan_counter_count_style(plan, theme),
        ),
        Span::styled(" ", theme.muted()),
        Span::styled(
            plan_counter_symbol(plan),
            plan_counter_symbol_style(plan, theme),
        ),
        Span::styled(" │", theme.muted()),
    ]))
}

pub(super) fn plan_counter_area(composer_area: Rect, plan: &PlanPanelState) -> Option<Rect> {
    if plan.is_empty() || composer_area.width < 8 || composer_area.height == 0 {
        return None;
    }
    let width = plan_counter_width(plan).min(composer_area.width.saturating_sub(1));
    Some(Rect::new(
        composer_area
            .x
            .saturating_add(composer_area.width.saturating_sub(width + 1)),
        composer_area.y,
        width,
        1,
    ))
}

fn parse_plan_items(output: &str) -> Vec<PlanItem> {
    output
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("- ")?;
            let (status, step) = rest.split_once(": ")?;
            let status = parse_status(status.trim())?;
            let step = step.trim();
            (!step.is_empty()).then(|| PlanItem {
                step: step.to_string(),
                status,
            })
        })
        .collect()
}

fn parse_status(status: &str) -> Option<PlanStatus> {
    match status {
        "pending" => Some(PlanStatus::Pending),
        "in_progress" => Some(PlanStatus::InProgress),
        "completed" => Some(PlanStatus::Completed),
        "blocked" => Some(PlanStatus::Blocked),
        _ => None,
    }
}

fn status_marker(status: PlanStatus, theme: Theme) -> (&'static str, ratatui::style::Style) {
    match status {
        PlanStatus::Pending => ("◇ ", theme.subtle()),
        PlanStatus::InProgress => ("◆ ", theme.running()),
        PlanStatus::Completed => (
            "✓ ",
            theme.policy_mode(roder_api::policy_mode::PolicyMode::AcceptAll),
        ),
        PlanStatus::Blocked => ("× ", theme.error()),
    }
}

fn plan_counter_width(plan: &PlanPanelState) -> u16 {
    // "│ " + count + " " + symbol + " │"
    (plan.len().to_string().chars().count() + 5) as u16
}

fn plan_counter_symbol(plan: &PlanPanelState) -> &'static str {
    if !plan.is_empty() && plan.completed_count() == plan.len() {
        "✓"
    } else if plan
        .items
        .iter()
        .any(|item| item.status == PlanStatus::Blocked)
    {
        "×"
    } else if plan
        .items
        .iter()
        .any(|item| item.status == PlanStatus::InProgress)
    {
        "◆"
    } else {
        "◇"
    }
}

fn plan_counter_count_style(plan: &PlanPanelState, theme: Theme) -> ratatui::style::Style {
    if !plan.is_empty() && plan.completed_count() == plan.len() {
        theme.policy_mode(roder_api::policy_mode::PolicyMode::AcceptAll)
    } else {
        theme.accent()
    }
}

fn plan_counter_symbol_style(plan: &PlanPanelState, theme: Theme) -> ratatui::style::Style {
    match plan_counter_symbol(plan) {
        "✓" => theme.policy_mode(roder_api::policy_mode::PolicyMode::AcceptAll),
        "◆" => theme.running(),
        "×" => theme.error(),
        _ => theme.subtle(),
    }
}

fn truncate_middle(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    let head = keep / 2;
    let tail = keep.saturating_sub(head);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_update_plan_output_into_items() {
        let output = "I will do this.\n- completed: Inspect current UI\n- in_progress: Render todos\n- pending: Verify";

        let items = parse_plan_items(output);

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].status, PlanStatus::Completed);
        assert_eq!(items[1].status, PlanStatus::InProgress);
        assert_eq!(items[2].step, "Verify");
    }

    #[test]
    fn update_plan_output_replaces_visibility_and_count() {
        let mut state = PlanPanelState::default();

        state.replace_from_update_plan_output("- completed: One\n- completed: Two");

        assert!(state.is_visible());
        assert_eq!(state.len(), 2);
        assert_eq!(state.completed_count(), 2);

        state.toggle();
        assert!(!state.is_visible());
    }

    #[test]
    fn task_ledger_output_reuses_plan_panel_and_blocked_status() {
        let mut state = PlanPanelState::default();

        state.replace_from_task_ledger_output(
            "Task ledger: 1/3 completed\n- completed: Inspect [inspect]\n- in_progress: Build [build]\n- blocked: Verify [verify]",
        );

        assert!(state.is_visible());
        assert_eq!(state.len(), 3);
        assert_eq!(state.completed_count(), 1);
        assert_eq!(state.items[2].status, PlanStatus::Blocked);
    }

    #[test]
    fn plan_counter_area_sits_on_composer_top_right() {
        let mut state = PlanPanelState::default();
        state.replace_from_update_plan_output(
            "- completed: One\n- completed: Two\n- completed: Three",
        );

        let area = plan_counter_area(Rect::new(10, 20, 80, 4), &state).unwrap();

        assert_eq!(area.y, 20);
        assert_eq!(area.height, 1);
        assert_eq!(area.x + area.width, 89);
    }
}
