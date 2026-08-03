//! Multi-select panel for the findings of a completed review.
//!
//! Roder has no multi-select widget, so the state machine is modelled on the
//! diff viewer's keymap (`crate::diff::keys`): navigation moves a cursor,
//! space toggles the item under it, `a`/`d` apply to every item at once, and
//! the panel returns a small action enum instead of touching the app itself.
//!
//! Theming tokens for this surface (RFC §"Class/ID Registry"):
//! `#review-panel`, `.review-finding`, `.review-finding-kept`, and
//! `.review-finding[data-priority="p0".."p3"]`. Like the other overlay panels
//! it renders through the [`Theme`] helpers rather than the cascade directly,
//! so it needs no new CSS variables.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use roder_api::review::{ReviewFinding, ReviewOutput, ReviewPriority};
use roder_app_server::AppClient;
use roder_protocol::{JsonRpcRequest, ReviewPublishParams, ReviewPublishResult};

use super::{Theme, TuiApp, decode_response, truncate};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum ReviewPanelView {
    List,
    Detail,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) enum ReviewPanelStatus {
    Ready,
    Empty,
    Error(String),
    Published(String),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum ReviewPanelAction {
    Close,
    Handled,
    Publish,
}

#[derive(Debug, Clone)]
pub(super) struct ReviewPanelState {
    review_id: String,
    output: ReviewOutput,
    /// Parallel to `output.findings`: whether the finding survives publishing.
    kept: Vec<bool>,
    list_state: ListState,
    view: ReviewPanelView,
    status: ReviewPanelStatus,
}

impl ReviewPanelState {
    pub(super) fn new(review_id: String, output: ReviewOutput) -> Self {
        let count = output.findings.len();
        let mut list_state = ListState::default();
        list_state.select((count > 0).then_some(0));
        let status = if count == 0 {
            ReviewPanelStatus::Empty
        } else {
            ReviewPanelStatus::Ready
        };
        Self {
            review_id,
            output,
            // Everything starts kept: the user drops noise rather than
            // rebuilding the reviewer's list by hand.
            kept: vec![true; count],
            list_state,
            view: ReviewPanelView::List,
            status,
        }
    }

    pub(super) fn review_id(&self) -> &str {
        &self.review_id
    }

    /// Indexes of the findings still marked keep, in report order.
    pub(super) fn kept_indexes(&self) -> Vec<usize> {
        self.kept
            .iter()
            .enumerate()
            .filter_map(|(index, kept)| kept.then_some(index))
            .collect()
    }

    pub(super) fn set_error(&mut self, message: impl Into<String>) {
        self.status = ReviewPanelStatus::Error(message.into());
    }

    pub(super) fn set_published(&mut self, message: impl Into<String>) {
        self.status = ReviewPanelStatus::Published(message.into());
    }

    fn selected(&self) -> Option<usize> {
        self.list_state
            .selected()
            .filter(|index| *index < self.output.findings.len())
    }

    fn selected_finding(&self) -> Option<&ReviewFinding> {
        self.output.findings.get(self.selected()?)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.output.findings.is_empty() {
            self.list_state.select(None);
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        let last = self.output.findings.len().saturating_sub(1) as isize;
        let next = (current as isize + delta).clamp(0, last) as usize;
        self.list_state.select(Some(next));
    }

    fn toggle_selected(&mut self) {
        if let Some(index) = self.selected() {
            self.kept[index] = !self.kept[index];
            self.status = ReviewPanelStatus::Ready;
        }
    }

    fn set_all(&mut self, kept: bool) {
        self.kept.fill(kept);
        if !self.output.findings.is_empty() {
            self.status = ReviewPanelStatus::Ready;
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> ReviewPanelAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return ReviewPanelAction::Handled;
        }
        match key.code {
            KeyCode::Esc => {
                if self.view == ReviewPanelView::Detail {
                    self.view = ReviewPanelView::List;
                    ReviewPanelAction::Handled
                } else {
                    ReviewPanelAction::Close
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                ReviewPanelAction::Handled
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                ReviewPanelAction::Handled
            }
            KeyCode::PageUp => {
                self.move_selection(-10);
                ReviewPanelAction::Handled
            }
            KeyCode::PageDown => {
                self.move_selection(10);
                ReviewPanelAction::Handled
            }
            KeyCode::Home => {
                self.move_selection(isize::MIN / 2);
                ReviewPanelAction::Handled
            }
            KeyCode::End => {
                self.move_selection(isize::MAX / 2);
                ReviewPanelAction::Handled
            }
            KeyCode::Char(' ') => {
                self.toggle_selected();
                ReviewPanelAction::Handled
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.set_all(true);
                ReviewPanelAction::Handled
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.set_all(false);
                ReviewPanelAction::Handled
            }
            KeyCode::Enter => {
                if self.selected().is_some() {
                    self.view = match self.view {
                        ReviewPanelView::List => ReviewPanelView::Detail,
                        ReviewPanelView::Detail => ReviewPanelView::List,
                    };
                }
                ReviewPanelAction::Handled
            }
            KeyCode::Char('p') | KeyCode::Char('P') => ReviewPanelAction::Publish,
            _ => ReviewPanelAction::Handled,
        }
    }
}

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn handle_review_panel_key(&mut self, key: KeyEvent) {
        let Some(action) = self
            .review_panel
            .as_mut()
            .map(|panel| panel.handle_key(key))
        else {
            return;
        };
        match action {
            ReviewPanelAction::Close => self.review_panel = None,
            ReviewPanelAction::Handled => {}
            ReviewPanelAction::Publish => self.publish_kept_review_findings().await,
        }
    }

    async fn publish_kept_review_findings(&mut self) {
        let Some((review_id, finding_indexes)) = self
            .review_panel
            .as_ref()
            .map(|panel| (panel.review_id().to_string(), panel.kept_indexes()))
        else {
            return;
        };
        if finding_indexes.is_empty() {
            if let Some(panel) = self.review_panel.as_mut() {
                panel.set_error("no findings are kept; press space or a before publishing");
            }
            return;
        }
        let params = ReviewPublishParams {
            review_id,
            publisher_id: self.review_publisher.clone(),
            finding_indexes: Some(finding_indexes),
            destination: None,
            dry_run: false,
        };
        match review_publish(&self.client, params).await {
            Ok(result) => {
                let summary = publish_summary(&result);
                if let Some(panel) = self.review_panel.as_mut() {
                    panel.set_published(summary.clone());
                }
                self.timeline.push_system(summary.clone());
                self.push_event(format!("review published: {}", result.publisher_id));
            }
            Err(err) => {
                if let Some(panel) = self.review_panel.as_mut() {
                    panel.set_error(format!("review/publish failed: {err}"));
                }
            }
        }
    }

    pub(super) fn render_review_panel(&mut self, f: &mut Frame<'_>, area: Rect) {
        let theme = self.theme;
        let Some(panel) = self.review_panel.as_mut() else {
            return;
        };
        render_review_panel(f, area, panel, theme);
    }
}

async fn review_publish<C: AppClient>(
    client: &C,
    params: ReviewPublishParams,
) -> anyhow::Result<ReviewPublishResult> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("review/publish")),
            method: "review/publish".to_string(),
            params: Some(serde_json::to_value(params)?),
        })
        .await;
    decode_response(res)
}

fn publish_summary(result: &ReviewPublishResult) -> String {
    let mut summary = format!(
        "Published {} finding(s) via {}",
        result.published_findings, result.publisher_id
    );
    if !result.skipped.is_empty() {
        summary.push_str(&format!(" ({} skipped)", result.skipped.len()));
    }
    if let Some(url) = result.url.as_deref() {
        summary.push_str(&format!(" — {url}"));
    }
    summary.push('.');
    summary
}

/// Durable transcript record of a review, so the findings survive closing the
/// panel.
pub(super) fn findings_transcript(label: &str, output: &ReviewOutput) -> String {
    let mut lines = vec![format!("Review of {label}:")];
    if let Some(explanation) = output.overall_explanation.as_deref() {
        lines.push(explanation.trim().to_string());
    }
    if output.findings.is_empty() {
        lines.push("No findings.".to_string());
        return lines.join("\n");
    }
    for finding in &output.findings {
        lines.push(format!(
            "- [{}] {} — {}",
            finding.priority.as_str().to_uppercase(),
            finding.display_title(),
            location_label(finding)
        ));
    }
    lines.join("\n")
}

fn location_label(finding: &ReviewFinding) -> String {
    let path = finding.code_location.absolute_file_path.display();
    let range = finding.code_location.line_range;
    if range.start == range.end {
        format!("{path}:{}", range.start)
    } else {
        format!("{path}:{}-{}", range.start, range.end)
    }
}

fn priority_style(priority: ReviewPriority, theme: Theme) -> Style {
    match priority {
        ReviewPriority::P0 => theme.error(),
        ReviewPriority::P1 => theme.shell(),
        ReviewPriority::P2 => theme.accent_soft(),
        ReviewPriority::P3 => theme.muted(),
    }
}

pub(super) fn render_review_panel(
    f: &mut Frame<'_>,
    area: Rect,
    state: &mut ReviewPanelState,
    theme: Theme,
) {
    let dialog_area = centered_rect(area);
    f.render_widget(Clear, dialog_area);

    let borders = if theme.borders_visible {
        Borders::ALL
    } else {
        Borders::NONE
    };
    let block = Block::default()
        .borders(borders)
        .border_type(theme.border_type)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .title(Span::styled(
            " Review findings (Space keep/drop, a all, d none, Enter detail, p publish, Esc close) ",
            theme.accent(),
        ));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);
    match state.view {
        ReviewPanelView::List => render_finding_list(f, chunks[0], state, theme),
        ReviewPanelView::Detail => render_finding_detail(f, chunks[0], state, theme),
    }
    f.render_widget(status_line(state, theme), chunks[1]);
}

fn render_finding_list(f: &mut Frame<'_>, area: Rect, state: &mut ReviewPanelState, theme: Theme) {
    if state.output.findings.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "The reviewer reported no findings.",
                theme.muted(),
            )))
            .style(theme.dialog_surface())
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let width = area.width.saturating_sub(16).max(12) as usize;
    let items = state
        .output
        .findings
        .iter()
        .enumerate()
        .map(|(index, finding)| {
            let kept = state.kept.get(index).copied().unwrap_or(true);
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(if kept { "[x] " } else { "[ ] " }, theme.accent_soft()),
                    Span::styled(
                        format!("[{}] ", finding.priority.as_str().to_uppercase()),
                        priority_style(finding.priority, theme),
                    ),
                    Span::styled(
                        truncate(finding.display_title(), width),
                        if kept { theme.text() } else { theme.muted() },
                    ),
                ]),
                Line::from(Span::styled(
                    format!("      {}", truncate(&location_label(finding), width)),
                    theme.subtle(),
                )),
            ])
        })
        .collect::<Vec<_>>();

    let list = List::new(items)
        .style(theme.dialog_surface())
        .highlight_style(theme.selected())
        .highlight_symbol("› ");
    f.render_stateful_widget(list, area, &mut state.list_state);
}

fn render_finding_detail(f: &mut Frame<'_>, area: Rect, state: &ReviewPanelState, theme: Theme) {
    let Some(finding) = state.selected_finding() else {
        return;
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("[{}] ", finding.priority.as_str().to_uppercase()),
                priority_style(finding.priority, theme),
            ),
            Span::styled(finding.display_title().to_string(), theme.strong()),
        ]),
        Line::from(Span::styled(location_label(finding), theme.subtle())),
        Line::from(""),
    ];
    lines.extend(
        finding
            .body
            .lines()
            .map(|line| Line::from(Span::styled(line.to_string(), theme.text()))),
    );
    if let Some(confidence) = finding.confidence_score {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("confidence {confidence:.2}"),
            theme.muted(),
        )));
    }
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .style(theme.dialog_surface())
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn status_line(state: &ReviewPanelState, theme: Theme) -> Paragraph<'static> {
    let (text, style) = match &state.status {
        ReviewPanelStatus::Ready => (
            format!(
                "{} of {} findings kept.",
                state.kept_indexes().len(),
                state.output.findings.len()
            ),
            theme.muted(),
        ),
        ReviewPanelStatus::Empty => (
            "Nothing to publish. Press Esc to close.".to_string(),
            theme.muted(),
        ),
        ReviewPanelStatus::Error(message) => (message.clone(), theme.error()),
        ReviewPanelStatus::Published(message) => (message.clone(), theme.accent_soft()),
    };
    Paragraph::new(Line::from(Span::styled(text, style))).style(theme.dialog_surface())
}

fn centered_rect(area: Rect) -> Rect {
    let width = ((area.width as u32 * 80) / 100).max(40) as u16;
    let height = ((area.height as u32 * 70) / 100).max(10) as u16;
    Rect {
        x: area.x + area.width.saturating_sub(width.min(area.width)) / 2,
        y: area.y + area.height.saturating_sub(height.min(area.height)) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::review::{ReviewCodeLocation, ReviewLineRange};

    fn finding(title: &str, priority: ReviewPriority, start: u32, end: u32) -> ReviewFinding {
        ReviewFinding {
            title: title.to_string(),
            body: "why this is wrong".to_string(),
            confidence_score: Some(0.8),
            priority,
            code_location: ReviewCodeLocation {
                absolute_file_path: "/repo/src/lib.rs".into(),
                line_range: ReviewLineRange { start, end },
            },
        }
    }

    fn panel() -> ReviewPanelState {
        ReviewPanelState::new(
            "review-1".to_string(),
            ReviewOutput {
                findings: vec![
                    finding("first", ReviewPriority::P0, 10, 12),
                    finding("second", ReviewPriority::P2, 40, 40),
                ],
                overall_correctness: Some("patch is incorrect".to_string()),
                overall_explanation: Some("two problems".to_string()),
                overall_confidence_score: Some(0.7),
            },
        )
    }

    fn press(state: &mut ReviewPanelState, code: KeyCode) -> ReviewPanelAction {
        state.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn every_finding_starts_kept() {
        assert_eq!(panel().kept_indexes(), vec![0, 1]);
    }

    #[test]
    fn space_toggles_only_the_selected_finding() {
        let mut state = panel();
        press(&mut state, KeyCode::Char(' '));
        assert_eq!(state.kept_indexes(), vec![1]);
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Char(' '));
        assert!(state.kept_indexes().is_empty());
        press(&mut state, KeyCode::Char(' '));
        assert_eq!(state.kept_indexes(), vec![1]);
    }

    #[test]
    fn keep_all_and_drop_all_apply_to_every_finding() {
        let mut state = panel();
        press(&mut state, KeyCode::Char('d'));
        assert!(state.kept_indexes().is_empty());
        press(&mut state, KeyCode::Char('a'));
        assert_eq!(state.kept_indexes(), vec![0, 1]);
    }

    #[test]
    fn navigation_clamps_at_both_ends() {
        let mut state = panel();
        press(&mut state, KeyCode::Up);
        assert_eq!(state.selected(), Some(0));
        press(&mut state, KeyCode::End);
        assert_eq!(state.selected(), Some(1));
        press(&mut state, KeyCode::Down);
        assert_eq!(state.selected(), Some(1));
        press(&mut state, KeyCode::Home);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn enter_opens_detail_and_escape_returns_to_the_list_before_closing() {
        let mut state = panel();
        assert_eq!(
            press(&mut state, KeyCode::Enter),
            ReviewPanelAction::Handled
        );
        assert_eq!(state.view, ReviewPanelView::Detail);
        assert_eq!(press(&mut state, KeyCode::Esc), ReviewPanelAction::Handled);
        assert_eq!(state.view, ReviewPanelView::List);
        assert_eq!(press(&mut state, KeyCode::Esc), ReviewPanelAction::Close);
    }

    #[test]
    fn p_requests_a_publish() {
        let mut state = panel();
        assert_eq!(
            press(&mut state, KeyCode::Char('p')),
            ReviewPanelAction::Publish
        );
    }

    #[test]
    fn empty_review_reports_nothing_to_publish() {
        let state = ReviewPanelState::new("review-2".to_string(), ReviewOutput::default());
        assert_eq!(state.status, ReviewPanelStatus::Empty);
        assert!(state.kept_indexes().is_empty());
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn transcript_lists_every_finding_with_its_location() {
        let state = panel();
        let transcript = findings_transcript("uncommitted changes", &state.output);
        assert!(transcript.contains("Review of uncommitted changes:"));
        assert!(transcript.contains("- [P0] first — /repo/src/lib.rs:10-12"));
        assert!(transcript.contains("- [P2] second — /repo/src/lib.rs:40"));
    }

    #[test]
    fn transcript_of_a_clean_review_says_so() {
        let transcript = findings_transcript("commit abc", &ReviewOutput::default());
        assert!(transcript.contains("No findings."));
    }
}
