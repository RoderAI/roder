use std::time::{Duration, Instant};

use crossterm::event::{
    KeyModifiers as CrosstermKeyModifiers, MouseButton as CrosstermMouseButton, MouseEvent,
    MouseEventKind,
};
use roder_api::interactive::{InteractiveEvent, KeyModifiers, MouseButton, RegionId};

use crate::mouse::regions::RegionFrame;

#[derive(Debug, Clone)]
pub struct MouseRouter {
    frame: RegionFrame,
    hover_region: Option<RegionId>,
    press: Option<PressState>,
    last_click: Option<(RegionId, Instant)>,
    double_click_window: Duration,
    drag_threshold: u16,
}

#[derive(Debug, Clone)]
struct PressState {
    region: RegionId,
    button: MouseButton,
    anchor: (u16, u16),
    dragging: bool,
}

impl Default for MouseRouter {
    fn default() -> Self {
        Self {
            frame: RegionFrame::default(),
            hover_region: None,
            press: None,
            last_click: None,
            double_click_window: Duration::from_millis(350),
            drag_threshold: 3,
        }
    }
}

impl MouseRouter {
    pub fn new(frame: RegionFrame) -> Self {
        Self {
            frame,
            ..Self::default()
        }
    }

    pub fn set_frame(&mut self, frame: RegionFrame) {
        self.frame = frame;
    }

    pub fn with_double_click_window(mut self, window: Duration) -> Self {
        self.double_click_window = window;
        self
    }

    pub fn with_drag_threshold(mut self, threshold: u16) -> Self {
        self.drag_threshold = threshold;
        self
    }

    pub fn handle_mouse_event(&mut self, event: MouseEvent, now: Instant) -> Vec<InteractiveEvent> {
        match event.kind {
            MouseEventKind::Moved => self.handle_hover(event.column, event.row),
            MouseEventKind::Down(button) => {
                self.handle_down(button, event.column, event.row);
                self.handle_hover(event.column, event.row)
            }
            MouseEventKind::Drag(_) => self.handle_drag(event.column, event.row),
            MouseEventKind::Up(button) => {
                self.handle_up(button, event.column, event.row, event.modifiers, now)
            }
            MouseEventKind::ScrollUp => {
                self.handle_scroll(event.column, event.row, -1, event.modifiers)
            }
            MouseEventKind::ScrollDown => {
                self.handle_scroll(event.column, event.row, 1, event.modifiers)
            }
            MouseEventKind::ScrollLeft => {
                self.handle_scroll(event.column, event.row, -1, event.modifiers)
            }
            MouseEventKind::ScrollRight => {
                self.handle_scroll(event.column, event.row, 1, event.modifiers)
            }
        }
    }

    fn handle_hover(&mut self, x: u16, y: u16) -> Vec<InteractiveEvent> {
        if self.press.as_ref().is_some_and(|press| press.dragging) {
            return Vec::new();
        }
        let next = self.frame.hit_test(x, y).map(|region| region.id.clone());
        if next == self.hover_region {
            return Vec::new();
        }
        let mut events = Vec::new();
        if let Some(region) = self.hover_region.take() {
            events.push(InteractiveEvent::HoverLeave { region });
        }
        if let Some(region) = next.clone() {
            events.push(InteractiveEvent::HoverEnter { region });
        }
        self.hover_region = next;
        events
    }

    fn handle_down(&mut self, button: CrosstermMouseButton, x: u16, y: u16) {
        let Some(button) = mouse_button(button) else {
            self.press = None;
            return;
        };
        self.press = self.frame.hit_test(x, y).map(|region| PressState {
            region: region.id.clone(),
            button,
            anchor: (x, y),
            dragging: false,
        });
    }

    fn handle_drag(&mut self, x: u16, y: u16) -> Vec<InteractiveEvent> {
        let Some(press) = self.press.as_mut() else {
            return Vec::new();
        };
        if !press.dragging && manhattan_distance(press.anchor, (x, y)) < self.drag_threshold {
            return Vec::new();
        }
        if !press.dragging {
            press.dragging = true;
            return vec![InteractiveEvent::DragStart {
                region: press.region.clone(),
                anchor: press.anchor,
            }];
        }
        vec![InteractiveEvent::DragUpdate {
            region: press.region.clone(),
            cursor: (x, y),
        }]
    }

    fn handle_up(
        &mut self,
        button: CrosstermMouseButton,
        x: u16,
        y: u16,
        modifiers: CrosstermKeyModifiers,
        now: Instant,
    ) -> Vec<InteractiveEvent> {
        let Some(press) = self.press.take() else {
            return Vec::new();
        };
        let modifiers = key_modifiers(modifiers);
        if press.dragging {
            return vec![InteractiveEvent::DragEnd {
                region: press.region,
                cursor: (x, y),
            }];
        }
        if mouse_button(button) != Some(press.button) {
            return Vec::new();
        }
        let Some(region) = self.frame.hit_test(x, y) else {
            return Vec::new();
        };
        if region.id != press.region {
            return Vec::new();
        }
        if press.button == MouseButton::Right {
            return vec![InteractiveEvent::RightClick {
                region: region.id.clone(),
                modifiers,
            }];
        }

        let mut events = vec![InteractiveEvent::Click {
            region: region.id.clone(),
            modifiers,
            button: press.button,
        }];
        if self.last_click.as_ref().is_some_and(|(last_region, at)| {
            last_region == &region.id && now.duration_since(*at) <= self.double_click_window
        }) {
            events.push(InteractiveEvent::DoubleClick {
                region: region.id.clone(),
                modifiers,
            });
        }
        self.last_click = Some((region.id.clone(), now));
        events
    }

    fn handle_scroll(
        &self,
        x: u16,
        y: u16,
        delta_lines: i16,
        modifiers: CrosstermKeyModifiers,
    ) -> Vec<InteractiveEvent> {
        vec![InteractiveEvent::Scroll {
            region: self.frame.hit_test(x, y).map(|region| region.id.clone()),
            delta_lines,
            modifiers: key_modifiers(modifiers),
        }]
    }
}

fn mouse_button(button: CrosstermMouseButton) -> Option<MouseButton> {
    match button {
        CrosstermMouseButton::Left => Some(MouseButton::Left),
        CrosstermMouseButton::Right => Some(MouseButton::Right),
        CrosstermMouseButton::Middle => Some(MouseButton::Middle),
    }
}

fn key_modifiers(modifiers: CrosstermKeyModifiers) -> KeyModifiers {
    KeyModifiers {
        shift: modifiers.contains(CrosstermKeyModifiers::SHIFT),
        control: modifiers.contains(CrosstermKeyModifiers::CONTROL),
        alt: modifiers.contains(CrosstermKeyModifiers::ALT),
        super_key: modifiers.contains(CrosstermKeyModifiers::SUPER),
    }
}

fn manhattan_distance(a: (u16, u16), b: (u16, u16)) -> u16 {
    a.0.abs_diff(b.0).saturating_add(a.1.abs_diff(b.1))
}
