use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollSettings {
    pub acceleration_enabled: bool,
    pub fixed_rows_per_tick: f32,
}

impl Default for ScrollSettings {
    fn default() -> Self {
        Self {
            acceleration_enabled: true,
            fixed_rows_per_tick: 10.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScrollAccelState {
    settings: ScrollSettings,
    last_tick: Option<Instant>,
    last_direction: Option<ScrollDirection>,
    velocity: f32,
}

impl Default for ScrollAccelState {
    fn default() -> Self {
        Self::new(ScrollSettings::default())
    }
}

impl ScrollAccelState {
    const IDLE_RESET: Duration = Duration::from_millis(160);
    const MAX_MULTIPLIER: f32 = 2.4;

    pub fn new(settings: ScrollSettings) -> Self {
        Self {
            settings,
            last_tick: None,
            last_direction: None,
            velocity: settings.fixed_rows_per_tick,
        }
    }

    pub fn tick(&mut self, direction: ScrollDirection, now: Instant) -> isize {
        let rows = if self.settings.acceleration_enabled {
            self.accelerated_rows(direction, now)
        } else {
            self.settings.fixed_rows_per_tick
        };
        rows.round().max(1.0).min(isize::MAX as f32) as isize
    }

    pub fn reset(&mut self) {
        self.last_tick = None;
        self.last_direction = None;
        self.velocity = self.settings.fixed_rows_per_tick;
    }

    fn accelerated_rows(&mut self, direction: ScrollDirection, now: Instant) -> f32 {
        let base = self.settings.fixed_rows_per_tick.max(1.0);
        let max = base * Self::MAX_MULTIPLIER;
        let elapsed = self
            .last_tick
            .map(|last| now.saturating_duration_since(last));
        let direction_changed = self.last_direction.is_some_and(|last| last != direction);

        if direction_changed || elapsed.is_none_or(|dt| dt > Self::IDLE_RESET) {
            self.velocity = base;
        } else if let Some(dt) = elapsed {
            let idle_ms = Self::IDLE_RESET.as_millis() as f32;
            let dt_ms = dt.as_secs_f32() * 1000.0;
            let quickness = ((idle_ms - dt_ms) / idle_ms).clamp(0.0, 1.0);
            // Repeated trackpad/wheel events inside the reset window ramp up
            // quickly, while slow discrete ticks stay at the precise base step.
            self.velocity = (self.velocity + base * quickness * 0.35).min(max);
        }

        self.last_tick = Some(now);
        self.last_direction = Some(direction);
        self.velocity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acceleration_is_enabled_by_default() {
        assert!(ScrollSettings::default().acceleration_enabled);
    }

    #[test]
    fn fixed_speed_ignores_rapid_ticks() {
        let start = Instant::now();
        let mut accel = ScrollAccelState::new(ScrollSettings {
            acceleration_enabled: false,
            fixed_rows_per_tick: 7.0,
        });

        assert_eq!(accel.tick(ScrollDirection::Down, start), 7);
        assert_eq!(
            accel.tick(ScrollDirection::Down, start + Duration::from_millis(10)),
            7
        );
    }

    #[test]
    fn macos_style_scroll_accelerates_then_resets_after_idle() {
        let start = Instant::now();
        let mut accel = ScrollAccelState::new(ScrollSettings {
            acceleration_enabled: true,
            fixed_rows_per_tick: 10.0,
        });

        let first = accel.tick(ScrollDirection::Down, start);
        let second = accel.tick(ScrollDirection::Down, start + Duration::from_millis(20));
        let third = accel.tick(ScrollDirection::Down, start + Duration::from_millis(40));
        let after_idle = accel.tick(ScrollDirection::Down, start + Duration::from_millis(300));

        assert_eq!(first, 10);
        assert!(second > first);
        assert!(third >= second);
        assert_eq!(after_idle, first);
    }

    #[test]
    fn direction_change_resets_acceleration() {
        let start = Instant::now();
        let mut accel = ScrollAccelState::new(ScrollSettings {
            acceleration_enabled: true,
            fixed_rows_per_tick: 10.0,
        });

        let first = accel.tick(ScrollDirection::Down, start);
        let accelerated = accel.tick(ScrollDirection::Down, start + Duration::from_millis(20));
        let reversed = accel.tick(ScrollDirection::Up, start + Duration::from_millis(40));

        assert!(accelerated > first);
        assert_eq!(reversed, first);
    }
}
