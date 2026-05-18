use ratatui::{
    style::Modifier,
    text::{Line, Span},
};
use roder_api::plan_review::{HunkDiffLineKind, HunkRecord, HunkRollbackState};

use super::Theme;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct HunkTrackerRow {
    hunk: HunkRecord,
    rollback_confirming: bool,
}

impl HunkTrackerRow {
    pub fn new(hunk: HunkRecord) -> Self {
        Self {
            hunk,
            rollback_confirming: false,
        }
    }

    #[allow(dead_code)]
    pub fn start_rollback_confirmation(&mut self) {
        self.rollback_confirming = true;
    }

    pub fn hunk_id(&self) -> &str {
        &self.hunk.id
    }

    pub fn can_expand(&self) -> bool {
        !self.hunk.diff.is_empty()
    }

    pub fn render(
        &self,
        selected: bool,
        expanded: bool,
        theme: Theme,
        lines: &mut Vec<Line<'static>>,
    ) {
        let title = format!(
            "hunk {} {}",
            self.hunk.id,
            rollback_label(&self.hunk.rollback)
        );
        lines.push(Line::from(vec![
            Span::styled("  Δ ", theme.tool()),
            Span::styled(
                format!("{} in {}", title, self.hunk.path),
                if selected {
                    theme.text().add_modifier(Modifier::BOLD)
                } else {
                    theme.text()
                },
            ),
        ]));
        if self.rollback_confirming {
            lines.push(Line::from(Span::styled(
                "    confirm rollback? y / n",
                theme.error().add_modifier(Modifier::BOLD),
            )));
        }
        if expanded {
            for diff in self.hunk.diff.iter().take(24) {
                let (prefix, style) = match diff.kind {
                    HunkDiffLineKind::Context => (" ", theme.muted()),
                    HunkDiffLineKind::Added => ("+", theme.tool()),
                    HunkDiffLineKind::Removed => ("-", theme.error()),
                };
                lines.push(Line::from(vec![
                    Span::styled("    ", theme.subtle()),
                    Span::styled(prefix.to_string(), style),
                    Span::styled(diff.text.clone(), style),
                ]));
            }
        }
    }
}

fn rollback_label(state: &HunkRollbackState) -> &'static str {
    match state {
        HunkRollbackState::Available => "rollback available",
        HunkRollbackState::Applied => "rolled back",
        HunkRollbackState::Conflict { .. } => "rollback conflict",
        HunkRollbackState::Unavailable { .. } => "rollback unavailable",
    }
}
