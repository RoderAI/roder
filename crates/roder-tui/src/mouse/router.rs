use std::time::{Duration, Instant};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use roder_api::interactive::{
    InteractiveEvent, InteractiveModifiers, InteractiveMouseButton, RegionId,
};

use super::handlers::{RegionHandlerDispatcher, RoutedInteractiveEvent};
use super::regions::RegionFrame;

const DEFAULT_DOUBLE_CLICK: Duration = Duration::from_millis(350);
const DEFAULT_DRAG_THRESHOLD: u16 = 3;
const DEFAULT_SCROLL_LINES: i16 = 3;
const CTRL_SCROLL_MULTIPLIER: i16 = 5;

#[derive(Debug, Clone)]
pub struct MouseRouter {
    hover_region: Option<RegionId>,
    press: Option<PressState>,
    last_click: Option<ClickState>,
    double_click: Duration,
    drag_threshold: u16,
    scroll_lines: i16,
}

#[derive(Debug, Clone)]
struct PressState {
    region: Option<RegionId>,
    button: InteractiveMouseButton,
    anchor: (u16, u16),
    modifiers: InteractiveModifiers,
    dragging: bool,
}

#[derive(Debug, Clone)]
struct ClickState {
    region: RegionId,
    at: Instant,
}

impl Default for MouseRouter {
    fn default() -> Self {
        Self {
            hover_region: None,
            press: None,
            last_click: None,
            double_click: DEFAULT_DOUBLE_CLICK,
            drag_threshold: DEFAULT_DRAG_THRESHOLD,
            scroll_lines: DEFAULT_SCROLL_LINES,
        }
    }
}

impl MouseRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_scroll_lines_per_tick(mut self, lines: i16) -> Self {
        self.scroll_lines = lines.max(1);
        self
    }

    pub fn process(&mut self, frame: &RegionFrame, event: MouseEvent) -> Vec<InteractiveEvent> {
        self.process_at(frame, event, Instant::now())
    }

    pub async fn process_and_dispatch(
        &mut self,
        frame: &RegionFrame,
        dispatcher: &RegionHandlerDispatcher,
        event: MouseEvent,
    ) -> anyhow::Result<Vec<RoutedInteractiveEvent>> {
        self.process_at_and_dispatch(frame, dispatcher, event, Instant::now())
            .await
    }

    pub fn process_at(
        &mut self,
        frame: &RegionFrame,
        event: MouseEvent,
        now: Instant,
    ) -> Vec<InteractiveEvent> {
        let region = frame
            .hit_test(event.column, event.row)
            .map(|region| region.id.clone());
        let mut out = Vec::new();

        match event.kind {
            MouseEventKind::Moved => self.push_hover_change(region, &mut out),
            MouseEventKind::Down(button) => {
                self.push_hover_change(region.clone(), &mut out);
                if let Some(button) = map_button(button) {
                    self.press = Some(PressState {
                        region,
                        button,
                        anchor: (event.column, event.row),
                        modifiers: map_modifiers(event.modifiers),
                        dragging: false,
                    });
                }
            }
            MouseEventKind::Drag(_) => {
                if let Some(press) = self.press.as_mut()
                    && let Some(region) = press.region.clone()
                {
                    let distance = event
                        .column
                        .abs_diff(press.anchor.0)
                        .max(event.row.abs_diff(press.anchor.1));
                    if !press.dragging && distance >= self.drag_threshold {
                        press.dragging = true;
                        out.push(InteractiveEvent::DragStart {
                            region: region.clone(),
                            anchor: press.anchor,
                        });
                    }
                    if press.dragging {
                        out.push(InteractiveEvent::DragUpdate {
                            region,
                            cursor: (event.column, event.row),
                        });
                    }
                }
            }
            MouseEventKind::Up(_) => {
                let Some(press) = self.press.take() else {
                    return out;
                };
                if let Some(pressed_region) = press.region {
                    if press.dragging {
                        out.push(InteractiveEvent::DragEnd {
                            region: pressed_region,
                            cursor: (event.column, event.row),
                        });
                    } else if region.as_deref() == Some(pressed_region.as_str()) {
                        self.push_click(
                            pressed_region,
                            press.button,
                            press.modifiers,
                            now,
                            &mut out,
                        );
                    }
                }
            }
            MouseEventKind::ScrollDown => out.push(InteractiveEvent::Scroll {
                region,
                delta_lines: scroll_delta(self.scroll_lines, event.modifiers, 1),
                modifiers: map_modifiers(event.modifiers),
            }),
            MouseEventKind::ScrollUp => out.push(InteractiveEvent::Scroll {
                region,
                delta_lines: scroll_delta(self.scroll_lines, event.modifiers, -1),
                modifiers: map_modifiers(event.modifiers),
            }),
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {}
        }

        out
    }

    pub async fn process_at_and_dispatch(
        &mut self,
        frame: &RegionFrame,
        dispatcher: &RegionHandlerDispatcher,
        event: MouseEvent,
        now: Instant,
    ) -> anyhow::Result<Vec<RoutedInteractiveEvent>> {
        let events = self.process_at(frame, event, now);
        let mut routed = Vec::with_capacity(events.len());
        for event in events {
            let region = event_region_id(&event).cloned();
            let outcome = match region.as_ref().and_then(|id| frame.get(id)) {
                Some(region) => dispatcher.dispatch(event.clone(), region).await?,
                None => roder_api::interactive::HandlerOutcome::Passthrough,
            };
            routed.push(RoutedInteractiveEvent {
                event,
                region,
                outcome,
            });
        }
        Ok(routed)
    }

    fn push_hover_change(&mut self, next: Option<RegionId>, out: &mut Vec<InteractiveEvent>) {
        if self.hover_region == next {
            return;
        }
        if let Some(region) = self.hover_region.take() {
            out.push(InteractiveEvent::HoverLeave { region });
        }
        if let Some(region) = next {
            self.hover_region = Some(region.clone());
            out.push(InteractiveEvent::HoverEnter { region });
        }
    }

    fn push_click(
        &mut self,
        region: RegionId,
        button: InteractiveMouseButton,
        modifiers: InteractiveModifiers,
        now: Instant,
        out: &mut Vec<InteractiveEvent>,
    ) {
        if button == InteractiveMouseButton::Right {
            out.push(InteractiveEvent::RightClick { region, modifiers });
            return;
        }
        let is_double = self.last_click.as_ref().is_some_and(|click| {
            click.region == region && now.duration_since(click.at) <= self.double_click
        });
        if is_double {
            out.push(InteractiveEvent::DoubleClick { region, modifiers });
            self.last_click = None;
        } else {
            out.push(InteractiveEvent::Click {
                region: region.clone(),
                modifiers,
                button,
            });
            self.last_click = Some(ClickState { region, at: now });
        }
    }
}

fn event_region_id(event: &InteractiveEvent) -> Option<&RegionId> {
    match event {
        InteractiveEvent::HoverEnter { region }
        | InteractiveEvent::HoverLeave { region }
        | InteractiveEvent::Click { region, .. }
        | InteractiveEvent::DoubleClick { region, .. }
        | InteractiveEvent::RightClick { region, .. }
        | InteractiveEvent::DragStart { region, .. }
        | InteractiveEvent::DragUpdate { region, .. }
        | InteractiveEvent::DragEnd { region, .. } => Some(region),
        InteractiveEvent::Scroll { region, .. } => region.as_ref(),
    }
}

fn scroll_delta(lines: i16, modifiers: KeyModifiers, direction: i16) -> i16 {
    let multiplier = if modifiers.contains(KeyModifiers::CONTROL) {
        CTRL_SCROLL_MULTIPLIER
    } else {
        1
    };
    direction * lines * multiplier
}

fn map_button(button: MouseButton) -> Option<InteractiveMouseButton> {
    match button {
        MouseButton::Left => Some(InteractiveMouseButton::Left),
        MouseButton::Right => Some(InteractiveMouseButton::Right),
        MouseButton::Middle => Some(InteractiveMouseButton::Middle),
    }
}

fn map_modifiers(modifiers: KeyModifiers) -> InteractiveModifiers {
    InteractiveModifiers {
        shift: modifiers.contains(KeyModifiers::SHIFT),
        control: modifiers.contains(KeyModifiers::CONTROL),
        alt: modifiers.contains(KeyModifiers::ALT),
        super_key: modifiers.contains(KeyModifiers::SUPER),
    }
}
