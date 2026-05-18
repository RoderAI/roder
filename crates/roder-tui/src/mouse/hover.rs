use ratatui::style::{Modifier, Style};
use roder_api::interactive::{HoverCursor, InteractiveEvent, RegionId};

use super::regions::RegionFrame;

#[derive(Debug, Clone)]
pub struct HoverState {
    active_region: Option<RegionId>,
    active_cursor: HoverCursor,
    dragging: bool,
    overlay: HoverStyleOverlay,
}

#[derive(Debug, Clone, Copy)]
pub struct HoverStyleOverlay {
    style: Style,
}

impl Default for HoverStyleOverlay {
    fn default() -> Self {
        Self {
            style: Style::default().add_modifier(Modifier::UNDERLINED | Modifier::BOLD),
        }
    }
}

impl HoverStyleOverlay {
    pub fn new(style: Style) -> Self {
        Self { style }
    }

    pub fn style(self) -> Style {
        self.style
    }
}

impl Default for HoverState {
    fn default() -> Self {
        Self {
            active_region: None,
            active_cursor: HoverCursor::Default,
            dragging: false,
            overlay: HoverStyleOverlay::default(),
        }
    }
}

impl HoverState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_region(&self) -> Option<&str> {
        self.active_region.as_deref()
    }

    pub fn active_cursor(&self) -> HoverCursor {
        if self.dragging {
            HoverCursor::Default
        } else {
            self.active_cursor
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn release_capture(&mut self) {
        self.active_region = None;
        self.active_cursor = HoverCursor::Default;
        self.dragging = false;
    }

    pub fn handle_event(&mut self, event: &InteractiveEvent, frame: &RegionFrame) {
        match event {
            InteractiveEvent::HoverEnter { region } => {
                self.active_region = Some(region.clone());
                self.active_cursor = frame
                    .get(region)
                    .map(|region| region.hover_cursor)
                    .unwrap_or(HoverCursor::Default);
            }
            InteractiveEvent::HoverLeave { region }
                if self.active_region.as_deref() == Some(region.as_str()) =>
            {
                self.active_region = None;
                self.active_cursor = HoverCursor::Default;
            }
            InteractiveEvent::DragStart { .. } => {
                self.dragging = true;
            }
            InteractiveEvent::DragEnd { .. } => {
                self.dragging = false;
            }
            _ => {}
        }
    }

    pub fn style_for(&self, region: &str) -> Option<Style> {
        if self.dragging || self.active_region.as_deref() != Some(region) {
            return None;
        }
        Some(self.overlay.style())
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;
    use roder_api::interactive::{HoverCursor, InteractiveRegion, RegionKind, RegionRect};

    use super::*;

    fn region(id: &str, cursor: HoverCursor) -> InteractiveRegion {
        InteractiveRegion {
            id: id.to_string(),
            rect: RegionRect {
                x: 0,
                y: 0,
                width: 5,
                height: 1,
            },
            z: 0,
            kind: RegionKind::Composer,
            hover_cursor: cursor,
            keyboard_binding: None,
        }
    }

    #[test]
    fn hover_enter_sets_region_cursor_and_style() {
        let mut frame = RegionFrame::new();
        frame.push(region("composer", HoverCursor::Text));
        let mut hover = HoverState::new();

        hover.handle_event(
            &InteractiveEvent::HoverEnter {
                region: "composer".to_string(),
            },
            &frame,
        );

        assert_eq!(hover.active_region(), Some("composer"));
        assert_eq!(hover.active_cursor(), HoverCursor::Text);
        assert!(
            hover
                .style_for("composer")
                .unwrap()
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
        assert!(hover.style_for("other").is_none());
    }

    #[test]
    fn hover_leave_clears_only_the_active_region() {
        let mut frame = RegionFrame::new();
        frame.push(region("status", HoverCursor::Pointer));
        let mut hover = HoverState::new();
        hover.handle_event(
            &InteractiveEvent::HoverEnter {
                region: "status".to_string(),
            },
            &frame,
        );
        hover.handle_event(
            &InteractiveEvent::HoverLeave {
                region: "other".to_string(),
            },
            &frame,
        );
        assert_eq!(hover.active_region(), Some("status"));

        hover.handle_event(
            &InteractiveEvent::HoverLeave {
                region: "status".to_string(),
            },
            &frame,
        );
        assert_eq!(hover.active_region(), None);
        assert_eq!(hover.active_cursor(), HoverCursor::Default);
    }

    #[test]
    fn hover_style_and_cursor_are_suppressed_during_drag() {
        let mut frame = RegionFrame::new();
        frame.push(region("message", HoverCursor::Pointer));
        let mut hover = HoverState::new();
        hover.handle_event(
            &InteractiveEvent::HoverEnter {
                region: "message".to_string(),
            },
            &frame,
        );
        hover.handle_event(
            &InteractiveEvent::DragStart {
                region: "message".to_string(),
                anchor: (0, 0),
            },
            &frame,
        );

        assert!(hover.is_dragging());
        assert_eq!(hover.active_cursor(), HoverCursor::Default);
        assert!(hover.style_for("message").is_none());

        hover.handle_event(
            &InteractiveEvent::DragEnd {
                region: "message".to_string(),
                cursor: (4, 0),
            },
            &frame,
        );
        assert!(!hover.is_dragging());
        assert_eq!(hover.active_cursor(), HoverCursor::Pointer);
        assert!(hover.style_for("message").is_some());
    }

    #[test]
    fn release_capture_clears_hover_and_drag_state() {
        let mut frame = RegionFrame::new();
        frame.push(region("composer", HoverCursor::Text));
        let mut hover = HoverState::new();
        hover.handle_event(
            &InteractiveEvent::HoverEnter {
                region: "composer".to_string(),
            },
            &frame,
        );
        hover.handle_event(
            &InteractiveEvent::DragStart {
                region: "composer".to_string(),
                anchor: (0, 0),
            },
            &frame,
        );

        hover.release_capture();

        assert_eq!(hover.active_region(), None);
        assert_eq!(hover.active_cursor(), HoverCursor::Default);
        assert!(!hover.is_dragging());
    }
}
