use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureEvent {
    CaptureEnabled,
    CaptureDisabled { until: Instant },
}

#[derive(Debug, Clone)]
pub struct CaptureController {
    enabled: bool,
    release_window: Duration,
    disabled_until: Option<Instant>,
}

impl Default for CaptureController {
    fn default() -> Self {
        Self {
            enabled: true,
            release_window: Duration::from_millis(600),
            disabled_until: None,
        }
    }
}

impl CaptureController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_release_window(mut self, release_window: Duration) -> Self {
        self.release_window = release_window;
        self
    }

    pub fn is_enabled(&self, now: Instant) -> bool {
        self.disabled_until.is_none_or(|until| now >= until)
    }

    pub fn tick(&mut self, now: Instant) -> Option<CaptureEvent> {
        if !self.enabled
            && let Some(until) = self.disabled_until
            && now >= until
        {
            self.enabled = true;
            self.disabled_until = None;
            return Some(CaptureEvent::CaptureEnabled);
        }
        None
    }

    pub fn handle_transcript_scroll(
        &mut self,
        now: Instant,
        current_scroll: usize,
        max_scroll: usize,
        delta_lines: i16,
    ) -> Option<CaptureEvent> {
        let at_top = current_scroll == 0 && delta_lines < 0;
        let at_bottom = current_scroll >= max_scroll && delta_lines > 0;
        if !(at_top || at_bottom) {
            return None;
        }

        let until = now + self.release_window;
        self.enabled = false;
        self.disabled_until = Some(until);
        Some(CaptureEvent::CaptureDisabled { until })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_releases_when_scroll_continues_past_top_or_bottom() {
        let now = Instant::now();
        let mut capture = CaptureController::new().with_release_window(Duration::from_millis(50));

        assert_eq!(
            capture.handle_transcript_scroll(now, 0, 10, -3),
            Some(CaptureEvent::CaptureDisabled {
                until: now + Duration::from_millis(50),
            })
        );
        assert!(!capture.is_enabled(now));
        assert_eq!(
            capture.tick(now + Duration::from_millis(50)),
            Some(CaptureEvent::CaptureEnabled)
        );
        assert!(capture.is_enabled(now + Duration::from_millis(50)));

        assert_eq!(
            capture.handle_transcript_scroll(now, 10, 10, 3),
            Some(CaptureEvent::CaptureDisabled {
                until: now + Duration::from_millis(50),
            })
        );
    }

    #[test]
    fn capture_stays_enabled_for_interior_scroll() {
        let now = Instant::now();
        let mut capture = CaptureController::new();

        assert_eq!(capture.handle_transcript_scroll(now, 5, 10, 3), None);
        assert!(capture.is_enabled(now));
    }

    #[test]
    fn rapid_boundary_scroll_extends_release_window() {
        let now = Instant::now();
        let mut capture = CaptureController::new().with_release_window(Duration::from_millis(100));

        let first = capture
            .handle_transcript_scroll(now, 0, 10, -3)
            .expect("first disable");
        let second = capture
            .handle_transcript_scroll(now + Duration::from_millis(80), 0, 10, -3)
            .expect("second disable");

        assert!(matches!(first, CaptureEvent::CaptureDisabled { .. }));
        assert_eq!(
            second,
            CaptureEvent::CaptureDisabled {
                until: now + Duration::from_millis(180),
            }
        );
        assert_eq!(capture.tick(now + Duration::from_millis(120)), None);
        assert_eq!(
            capture.tick(now + Duration::from_millis(180)),
            Some(CaptureEvent::CaptureEnabled)
        );
    }
}
