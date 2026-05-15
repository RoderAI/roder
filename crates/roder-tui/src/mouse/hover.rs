use roder_api::interactive::{HoverCursor, InteractiveEvent, InteractiveRegion, RegionId};

use crate::mouse::regions::RegionFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoverVisualStyle {
    pub underline: bool,
    pub bold: bool,
}

impl Default for HoverVisualStyle {
    fn default() -> Self {
        Self {
            underline: true,
            bold: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverStyleOverlay {
    pub region_id: RegionId,
    pub style: HoverVisualStyle,
}

#[derive(Debug, Clone, Default)]
pub struct HoverState {
    active: Option<HoveredRegion>,
    style: HoverVisualStyle,
    drag_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoveredRegion {
    pub id: RegionId,
    pub cursor: HoverCursor,
}

impl HoverState {
    pub fn apply_event(
        &mut self,
        event: &InteractiveEvent,
        frame: &RegionFrame,
    ) -> Option<HoveredRegion> {
        match event {
            InteractiveEvent::HoverEnter { region } => {
                let hovered = frame.get(region).map(hovered_region)?;
                self.active = Some(hovered.clone());
                Some(hovered)
            }
            InteractiveEvent::HoverLeave { region } => {
                if self
                    .active
                    .as_ref()
                    .is_some_and(|active| &active.id == region)
                {
                    self.active = None;
                }
                self.active.clone()
            }
            InteractiveEvent::DragStart { .. } => {
                self.drag_active = true;
                self.active.clone()
            }
            InteractiveEvent::DragEnd { .. } => {
                self.drag_active = false;
                self.active.clone()
            }
            _ => self.active.clone(),
        }
    }

    pub fn clear(&mut self) {
        self.active = None;
        self.drag_active = false;
    }

    pub fn active(&self) -> Option<&HoveredRegion> {
        self.active.as_ref()
    }

    pub fn overlay_for(&self, region_id: &str) -> Option<HoverStyleOverlay> {
        if self.drag_active {
            return None;
        }
        let active = self.active.as_ref()?;
        (active.id == region_id).then(|| HoverStyleOverlay {
            region_id: active.id.clone(),
            style: self.style,
        })
    }
}

fn hovered_region(region: &InteractiveRegion) -> HoveredRegion {
    HoveredRegion {
        id: region.id.clone(),
        cursor: region.hover_cursor,
    }
}

#[cfg(test)]
mod tests {
    use roder_api::interactive::{HoverCursor, InteractiveRegion, RegionKind, RegionRect};

    use super::*;

    #[test]
    fn hover_tracks_composer_region_and_style_overlay() {
        let frame = frame_with_regions();
        let mut state = HoverState::default();

        state.apply_event(
            &InteractiveEvent::HoverEnter {
                region: "composer".to_string(),
            },
            &frame,
        );

        assert_eq!(state.active().unwrap().id, "composer");
        assert_eq!(state.active().unwrap().cursor, HoverCursor::Text);
        assert_eq!(
            state.overlay_for("composer").unwrap().style,
            HoverVisualStyle::default()
        );
        assert!(state.overlay_for("status").is_none());
    }

    #[test]
    fn hover_clears_across_status_and_off_frame_edges() {
        let frame = frame_with_regions();
        let mut state = HoverState::default();

        state.apply_event(
            &InteractiveEvent::HoverEnter {
                region: "status".to_string(),
            },
            &frame,
        );
        assert_eq!(state.active().unwrap().cursor, HoverCursor::Pointer);

        state.apply_event(
            &InteractiveEvent::HoverLeave {
                region: "status".to_string(),
            },
            &frame,
        );
        assert!(state.active().is_none());
        assert!(state.overlay_for("status").is_none());
    }

    #[test]
    fn hover_style_is_suppressed_during_drag() {
        let frame = frame_with_regions();
        let mut state = HoverState::default();

        state.apply_event(
            &InteractiveEvent::HoverEnter {
                region: "composer".to_string(),
            },
            &frame,
        );
        state.apply_event(
            &InteractiveEvent::DragStart {
                region: "composer".to_string(),
                anchor: (1, 1),
            },
            &frame,
        );

        assert!(state.overlay_for("composer").is_none());

        state.apply_event(
            &InteractiveEvent::DragEnd {
                region: "composer".to_string(),
                cursor: (4, 1),
            },
            &frame,
        );
        assert!(state.overlay_for("composer").is_some());
    }

    fn frame_with_regions() -> RegionFrame {
        let mut builder = RegionFrame::builder();
        builder.push(region(
            "composer",
            HoverCursor::Text,
            RegionKind::Composer,
            0,
            8,
        ));
        builder.push(region(
            "status",
            HoverCursor::Pointer,
            RegionKind::StatusSegment {
                segment_id: "mode".to_string(),
            },
            9,
            1,
        ));
        builder.build()
    }

    fn region(
        id: &str,
        hover_cursor: HoverCursor,
        kind: RegionKind,
        y: u16,
        height: u16,
    ) -> InteractiveRegion {
        InteractiveRegion {
            id: id.to_string(),
            rect: RegionRect {
                x: 0,
                y,
                width: 20,
                height,
            },
            z: 0,
            kind,
            hover_cursor,
            keyboard_binding: None,
        }
    }
}
