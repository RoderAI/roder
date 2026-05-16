use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub(super) struct TurnTimer {
    started_at: Option<Instant>,
    paused_for: Duration,
    pause_started_at: Option<Instant>,
}

impl TurnTimer {
    pub(super) fn start(&mut self, now: Instant) {
        self.started_at = Some(now);
        self.paused_for = Duration::ZERO;
        self.pause_started_at = None;
    }

    pub(super) fn pause(&mut self, now: Instant) {
        if self.started_at.is_some() && self.pause_started_at.is_none() {
            self.pause_started_at = Some(now);
        }
    }

    pub(super) fn resume(&mut self, now: Instant) {
        if let Some(paused_at) = self.pause_started_at.take() {
            self.paused_for += now.saturating_duration_since(paused_at);
        }
    }

    pub(super) fn elapsed(&self, now: Instant) -> Duration {
        let Some(started_at) = self.started_at else {
            return Duration::ZERO;
        };
        let current_pause = self
            .pause_started_at
            .map(|paused_at| now.saturating_duration_since(paused_at))
            .unwrap_or_default();
        now.saturating_duration_since(started_at)
            .saturating_sub(self.paused_for + current_pause)
    }

    pub(super) fn finish(&mut self, now: Instant) -> Duration {
        let elapsed = self.elapsed(now);
        self.reset();
        elapsed
    }

    pub(super) fn reset(&mut self) {
        self.started_at = None;
        self.paused_for = Duration::ZERO;
        self.pause_started_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_excludes_active_approval_pause() {
        let start = Instant::now();
        let mut timer = TurnTimer::default();
        timer.start(start);
        timer.pause(start + Duration::from_secs(5));

        assert_eq!(
            timer.elapsed(start + Duration::from_secs(20)),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn elapsed_excludes_resolved_approval_pause() {
        let start = Instant::now();
        let mut timer = TurnTimer::default();
        timer.start(start);
        timer.pause(start + Duration::from_secs(5));
        timer.resume(start + Duration::from_secs(20));

        assert_eq!(
            timer.elapsed(start + Duration::from_secs(25)),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn finish_excludes_pause_and_resets_timer() {
        let start = Instant::now();
        let mut timer = TurnTimer::default();
        timer.start(start);
        timer.pause(start + Duration::from_secs(5));
        timer.resume(start + Duration::from_secs(20));

        assert_eq!(
            timer.finish(start + Duration::from_secs(30)),
            Duration::from_secs(15)
        );
        assert_eq!(
            timer.elapsed(start + Duration::from_secs(40)),
            Duration::ZERO
        );
    }
}
