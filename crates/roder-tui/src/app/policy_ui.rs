use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap},
};
use roder_api::interactive::{
    ApprovalVote, HoverCursor, InteractiveRegion, RegionKind, RegionRect,
};
use roder_protocol::PendingPlanExitDescriptor;

use super::{Theme, TuiApp, centered_rect, policy_mode_label};

impl TuiApp {
    pub(super) fn policy_approval_regions(&self, area: Rect) -> Vec<InteractiveRegion> {
        let Some(pending) = self.pending_plan_exit.as_ref() else {
            return Vec::new();
        };
        policy_approval_regions(pending, area)
    }
}

pub(super) fn render_policy_approval(
    f: &mut Frame<'_>,
    area: Rect,
    pending: &PendingPlanExitDescriptor,
    theme: Theme,
) {
    let dialog_area = policy_dialog_area(area);
    let shadow_area = shadow_rect(dialog_area, area);
    f.render_widget(Clear, shadow_area);
    f.render_widget(Paragraph::new("").style(theme.dialog_shadow()), shadow_area);
    f.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .padding(Padding::horizontal(2))
        .title(Span::styled(" Plan approval ", theme.accent()));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "policy request",
            theme.accent_soft(),
        )))
        .style(theme.dialog_surface()),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                "Exit plan mode into {}?",
                policy_mode_label(pending.target_mode)
            ),
            theme.strong(),
        )))
        .style(theme.dialog_surface()),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(Text::from(Line::from(Span::styled(
            pending.plan_summary.clone().unwrap_or_else(|| {
                "The model requested permission to continue with side effects.".to_string()
            }),
            theme.muted(),
        ))))
        .style(theme.dialog_surface())
        .wrap(Wrap { trim: true }),
        chunks[2],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Approve]", theme.dialog_key()),
            Span::styled("    ", theme.dialog_surface()),
            Span::styled("[Deny]", theme.muted()),
            Span::styled("    Y/N", theme.muted()),
        ]))
        .style(theme.dialog_surface()),
        chunks[3],
    );
}

pub(super) fn policy_approval_regions(
    pending: &PendingPlanExitDescriptor,
    area: Rect,
) -> Vec<InteractiveRegion> {
    let (approve, reject) = policy_button_rects(policy_dialog_area(area));
    [
        (ApprovalVote::Approve, approve, "approve"),
        (ApprovalVote::Reject, reject, "reject"),
    ]
    .into_iter()
    .map(|(vote, rect, label)| InteractiveRegion {
        id: format!("policy:{}:{label}", pending.request_id),
        rect,
        z: 30,
        kind: RegionKind::PolicyApprovalButton {
            decision_id: pending.request_id.clone(),
            vote,
        },
        hover_cursor: HoverCursor::Pointer,
        keyboard_binding: None,
    })
    .collect()
}

fn policy_dialog_area(area: Rect) -> Rect {
    centered_rect(area, dialog_width(area), 9.min(area.height))
}

fn policy_button_rects(dialog_area: Rect) -> (RegionRect, RegionRect) {
    let inner_x = dialog_area.x.saturating_add(3);
    let action_y = dialog_area
        .y
        .saturating_add(dialog_area.height.saturating_sub(2));
    (
        RegionRect {
            x: inner_x,
            y: action_y,
            width: 10,
            height: 1,
        },
        RegionRect {
            x: inner_x.saturating_add(14),
            y: action_y,
            width: 8,
            height: 1,
        },
    )
}

fn dialog_width(area: Rect) -> u16 {
    let roomy_width = area.width.saturating_sub(4).min(64);
    roomy_width.max(area.width.min(44))
}

fn shadow_rect(dialog_area: Rect, bounds: Rect) -> Rect {
    Rect {
        x: dialog_area.x.saturating_add(2).min(bounds.right()),
        y: dialog_area.y.saturating_add(1).min(bounds.bottom()),
        width: dialog_area.width.min(
            bounds
                .right()
                .saturating_sub(dialog_area.x.saturating_add(2)),
        ),
        height: dialog_area.height.min(
            bounds
                .bottom()
                .saturating_sub(dialog_area.y.saturating_add(1)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::{ThreadId, TurnId};
    use roder_api::policy_mode::PolicyMode;
    use time::OffsetDateTime;

    #[test]
    fn policy_approval_regions_cover_approve_and_reject_buttons() {
        let pending = pending_plan_exit();

        let regions = policy_approval_regions(&pending, Rect::new(0, 0, 80, 24));

        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].id, "policy:req-1:approve");
        assert_eq!(regions[1].id, "policy:req-1:reject");
        assert!(matches!(
            regions[0].kind,
            RegionKind::PolicyApprovalButton {
                vote: ApprovalVote::Approve,
                ..
            }
        ));
        assert!(
            regions[0]
                .rect
                .contains(regions[0].rect.x, regions[0].rect.y)
        );
    }

    fn pending_plan_exit() -> PendingPlanExitDescriptor {
        PendingPlanExitDescriptor {
            thread_id: ThreadId::from("thread"),
            turn_id: TurnId::from("turn"),
            request_id: "req-1".to_string(),
            target_mode: PolicyMode::Default,
            plan_summary: Some("Implement the plan.".to_string()),
            requested_at: OffsetDateTime::UNIX_EPOCH,
            expires_at: None,
        }
    }
}
