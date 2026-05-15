#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseCaptureEvent {
    CaptureEnabled,
    CaptureDisabled,
}

#[derive(Debug, Clone)]
pub struct MouseCaptureController {
    enabled: bool,
    disabled_until_tick: Option<u64>,
    release_ticks: u64,
}

impl Default for MouseCaptureController {
    fn default() -> Self {
        Self {
            enabled: true,
            disabled_until_tick: None,
            release_ticks: 8,
        }
    }
}

impl MouseCaptureController {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn release_for_boundary_scroll(&mut self, now_tick: u64) -> Option<MouseCaptureEvent> {
        self.disabled_until_tick = Some(now_tick.saturating_add(self.release_ticks));
        if self.enabled {
            self.enabled = false;
            Some(MouseCaptureEvent::CaptureDisabled)
        } else {
            None
        }
    }

    pub fn tick(&mut self, now_tick: u64) -> Option<MouseCaptureEvent> {
        if !self.enabled
            && self
                .disabled_until_tick
                .is_some_and(|until| now_tick >= until)
        {
            self.enabled = true;
            self.disabled_until_tick = None;
            return Some(MouseCaptureEvent::CaptureEnabled);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_releases_and_restores_after_window() {
        let mut controller = MouseCaptureController::default();

        assert_eq!(
            controller.release_for_boundary_scroll(10),
            Some(MouseCaptureEvent::CaptureDisabled)
        );
        assert!(!controller.enabled());
        assert_eq!(controller.tick(12), None);
        assert_eq!(controller.tick(18), Some(MouseCaptureEvent::CaptureEnabled));
        assert!(controller.enabled());
    }
}
