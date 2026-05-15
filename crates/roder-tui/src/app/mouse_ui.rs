use std::time::Instant;

use crossterm::event::MouseEvent;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
};
use roder_api::{
    interactive::{HoverCursor, InteractiveRegion, RegionKind},
    tui_status::{StatusCell, StatusSegment, StatusStyle},
};

use crate::mouse::{
    CursorFeedback, HoverState, MouseRouter, RegionFrame, region_rect_from_ratatui,
};

pub const COMPOSER_REGION_ID: &str = "composer";
pub const STATUS_REGION_ID: &str = "status-line";

#[derive(Debug, Default)]
pub struct MouseFeedbackState {
    router: MouseRouter,
    regions: RegionFrame,
    hover: HoverState,
    cursor: CursorFeedback,
}

impl MouseFeedbackState {
    pub fn set_frame_regions(&mut self, composer: Rect, status: Rect) {
        let mut builder = RegionFrame::builder();
        builder.push(region(
            COMPOSER_REGION_ID,
            RegionKind::Composer,
            HoverCursor::Text,
            composer,
            0,
        ));
        builder.push(region(
            STATUS_REGION_ID,
            RegionKind::StatusSegment {
                segment_id: "status-line".to_string(),
            },
            HoverCursor::Pointer,
            status,
            0,
        ));
        self.regions = builder.build();
        self.router.set_frame(self.regions.clone());
        if self
            .hover
            .active()
            .is_some_and(|active| self.regions.get(&active.id).is_none())
        {
            self.hover.clear();
            self.cursor.clear();
        }
    }

    pub fn handle_mouse_event(&mut self, event: MouseEvent, now: Instant) {
        for event in self.router.handle_mouse_event(event, now) {
            match self.hover.apply_event(&event, &self.regions) {
                Some(hovered) => {
                    self.cursor.update(hovered.cursor);
                }
                None => self.cursor.clear(),
            }
        }
    }

    pub fn style_for_region(&self, region_id: &str, base: Style) -> Style {
        let Some(overlay) = self.hover.overlay_for(region_id) else {
            return base;
        };
        let mut style = base;
        if overlay.style.underline {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        if overlay.style.bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        style
    }

    pub fn status_segment(&self) -> Option<StatusSegment> {
        let indicator = self.cursor.pointer_indicator()?.to_string();
        let hover_label = self
            .hover
            .active()
            .map(|hovered| hovered.id.clone())
            .unwrap_or_else(|| "interactive".to_string());
        Some(StatusSegment::new("mouse", 110, 7, move |_| StatusCell {
            text: format!("mouse:{indicator}"),
            style: StatusStyle::Accent,
            tooltip: Some(format!("Hover {hover_label}")),
        }))
    }
}

fn region(
    id: &str,
    kind: RegionKind,
    hover_cursor: HoverCursor,
    rect: Rect,
    z: i16,
) -> InteractiveRegion {
    InteractiveRegion {
        id: id.to_string(),
        rect: region_rect_from_ratatui(rect),
        z,
        kind,
        hover_cursor,
        keyboard_binding: None,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

    use super::*;

    #[test]
    fn hover_feedback_sets_pointer_segment_and_clears_off_frame() {
        let mut state = MouseFeedbackState::default();
        state.set_frame_regions(Rect::new(0, 0, 20, 3), Rect::new(0, 4, 20, 1));

        state.handle_mouse_event(mouse(MouseEventKind::Moved, 2, 4), Instant::now());
        let segment = state.status_segment().expect("status pointer indicator");
        let cell = (segment.render)(&empty_status_context());
        assert_eq!(cell.text, "mouse:*");

        state.handle_mouse_event(mouse(MouseEventKind::Moved, 25, 4), Instant::now());
        assert!(state.status_segment().is_none());
    }

    #[test]
    fn hover_feedback_styles_composer_region() {
        let mut state = MouseFeedbackState::default();
        state.set_frame_regions(Rect::new(0, 0, 20, 3), Rect::new(0, 4, 20, 1));

        state.handle_mouse_event(mouse(MouseEventKind::Moved, 2, 1), Instant::now());

        assert!(
            state
                .style_for_region(COMPOSER_REGION_ID, Style::default())
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::empty(),
        }
    }

    fn empty_status_context<'a>() -> roder_api::tui_status::StatusContext<'a> {
        use roder_api::{
            policy_mode::PolicyMode,
            tui_status::{McpServerStatus, SessionSummary, StatusContext},
        };

        let session = Box::leak(Box::new(SessionSummary {
            thread_id: "thread".to_string(),
            title: None,
        }));
        let mcp = Box::leak(Vec::<McpServerStatus>::new().into_boxed_slice());
        StatusContext {
            session,
            policy_mode: PolicyMode::Default,
            model: None,
            usage: None,
            git: None,
            mcp,
        }
    }
}
