use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use roder_api::plan_review::{PlanComment, PlanReview, PlanReviewStatus, PlanRewrite};

use super::Theme;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct PlanReviewRow {
    review: PlanReview,
}

impl PlanReviewRow {
    pub fn new(review: PlanReview) -> Self {
        Self { review }
    }

    pub fn update_status(&mut self, status: PlanReviewStatus) {
        self.review.status = status;
    }

    pub fn review_id(&self) -> &str {
        &self.review.id
    }

    pub fn can_expand(&self) -> bool {
        !self.review.markdown.trim().is_empty()
    }

    pub fn push_comment(&mut self, comment: PlanComment) {
        self.review.comments.push(comment);
    }

    pub fn push_rewrite(&mut self, rewrite: PlanRewrite) {
        self.review.markdown = rewrite.replacement_markdown.clone();
        self.review.rewrites.push(rewrite);
        self.review.status = PlanReviewStatus::Rewritten;
    }

    pub fn render(
        &self,
        selected: bool,
        expanded: bool,
        theme: Theme,
        lines: &mut Vec<Line<'static>>,
    ) {
        let style = if selected {
            theme.text().add_modifier(Modifier::BOLD)
        } else {
            theme.text()
        };
        lines.push(Line::from(vec![
            Span::styled("  ◇ ", status_style(&self.review.status, theme)),
            Span::styled(
                format!(
                    "plan review: {} ({})",
                    self.review.title,
                    status_label(&self.review.status)
                ),
                style,
            ),
        ]));
        for step in self.review.steps.iter().take(4) {
            let marker = if step.completed { "[x]" } else { "[ ]" };
            lines.push(Line::from(vec![
                Span::styled("    ", theme.subtle()),
                Span::styled(marker.to_string(), theme.subtle()),
                Span::styled(format!(" {}", step.title), theme.muted()),
            ]));
        }
        if !self.review.comments.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("    {} comments", self.review.comments.len()),
                theme.subtle(),
            )));
        }
        if expanded {
            for line in self.review.markdown.lines().take(12) {
                lines.push(Line::from(vec![
                    Span::styled("    ", theme.subtle()),
                    Span::styled(line.to_string(), theme.muted()),
                ]));
            }
        }
    }
}

fn status_label(status: &PlanReviewStatus) -> &'static str {
    match status {
        PlanReviewStatus::Drafted => "drafted",
        PlanReviewStatus::AwaitingReview => "awaiting review",
        PlanReviewStatus::Rewritten => "rewritten",
        PlanReviewStatus::Approved => "approved",
        PlanReviewStatus::Executing => "executing",
        PlanReviewStatus::Completed => "completed",
        PlanReviewStatus::Rejected => "rejected",
    }
}

fn status_style(status: &PlanReviewStatus, theme: Theme) -> Style {
    match status {
        PlanReviewStatus::Rejected => theme.error(),
        PlanReviewStatus::Approved | PlanReviewStatus::Completed => theme.tool(),
        _ => theme.accent_soft(),
    }
}
