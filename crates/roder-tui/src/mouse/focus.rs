use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use roder_api::interactive::RegionId;

use super::regions::RegionFrame;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionFocusController {
    focused: Option<RegionId>,
}

impl RegionFocusController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn focused(&self) -> Option<&RegionId> {
        self.focused.as_ref()
    }

    pub fn handle_key(&mut self, frame: &RegionFrame, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.focus_previous(frame)
            }
            KeyCode::Tab => self.focus_next(frame),
            _ => false,
        }
    }

    pub fn focus_next(&mut self, frame: &RegionFrame) -> bool {
        self.focus_by(frame, 1)
    }

    pub fn focus_previous(&mut self, frame: &RegionFrame) -> bool {
        self.focus_by(frame, -1)
    }

    fn focus_by(&mut self, frame: &RegionFrame, delta: isize) -> bool {
        let regions = frame
            .iter()
            .filter(|region| region.keyboard_binding.is_some())
            .map(|region| region.id.clone())
            .collect::<Vec<_>>();
        if regions.is_empty() {
            self.focused = None;
            return false;
        }

        let current = self
            .focused
            .as_ref()
            .and_then(|focused| regions.iter().position(|region| region == focused))
            .unwrap_or(if delta >= 0 { regions.len() - 1 } else { 0 });
        let next = (current as isize + delta).rem_euclid(regions.len() as isize) as usize;
        self.focused = Some(regions[next].clone());
        true
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use roder_api::interactive::{
        HoverCursor, InteractiveModifiers, InteractiveRegion, KeyChord, RegionKind, RegionRect,
    };

    use super::*;

    fn focusable(id: &str) -> InteractiveRegion {
        InteractiveRegion {
            id: id.to_string(),
            rect: RegionRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            z: 0,
            kind: RegionKind::Composer,
            hover_cursor: HoverCursor::Default,
            keyboard_binding: Some(KeyChord {
                key: "enter".to_string(),
                modifiers: InteractiveModifiers::default(),
            }),
        }
    }

    #[test]
    fn tab_and_shift_tab_traverse_focusable_regions() {
        let mut frame = RegionFrame::new();
        frame.push(focusable("first"));
        frame.push(focusable("second"));
        let mut focus = RegionFocusController::new();

        assert!(focus.handle_key(&frame, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(focus.focused().map(String::as_str), Some("first"));
        assert!(focus.handle_key(&frame, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(focus.focused().map(String::as_str), Some("second"));
        assert!(focus.handle_key(&frame, KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT)));
        assert_eq!(focus.focused().map(String::as_str), Some("first"));
    }
}
