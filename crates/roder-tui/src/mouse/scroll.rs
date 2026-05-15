use roder_api::interactive::KeyModifiers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollTarget {
    Transcript,
    Palette,
    Diff,
    Composer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollOutcome {
    pub target: ScrollTarget,
    pub offset: usize,
    pub at_boundary: bool,
}

#[derive(Debug, Clone)]
pub struct ScrollState {
    target: ScrollTarget,
    offset: usize,
    max_offset: usize,
    lines_per_tick: usize,
    fast_multiplier: usize,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            target: ScrollTarget::Transcript,
            offset: 0,
            max_offset: 0,
            lines_per_tick: 3,
            fast_multiplier: 5,
        }
    }
}

impl ScrollState {
    pub fn set_target(&mut self, target: ScrollTarget) {
        self.target = target;
    }

    pub fn set_bounds(&mut self, content_rows: usize, viewport_rows: usize) {
        self.max_offset = content_rows.saturating_sub(viewport_rows);
        self.offset = self.offset.min(self.max_offset);
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn scroll(&mut self, delta_lines: i16, modifiers: KeyModifiers) -> ScrollOutcome {
        let amount = self.scroll_amount(delta_lines, modifiers);
        let previous = self.offset;
        if delta_lines < 0 {
            self.offset = self.offset.saturating_sub(amount);
        } else if delta_lines > 0 {
            self.offset = self.offset.saturating_add(amount).min(self.max_offset);
        }
        ScrollOutcome {
            target: self.target,
            offset: self.offset,
            at_boundary: previous == self.offset && delta_lines != 0,
        }
    }

    fn scroll_amount(&self, delta_lines: i16, modifiers: KeyModifiers) -> usize {
        let ticks = usize::from(delta_lines.unsigned_abs()).max(1);
        let multiplier = if modifiers.control {
            self.fast_multiplier
        } else {
            1
        };
        ticks
            .saturating_mul(self.lines_per_tick)
            .saturating_mul(multiplier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_moves_inside_bounds() {
        let mut state = ScrollState::default();
        state.set_bounds(30, 10);

        let outcome = state.scroll(1, KeyModifiers::default());

        assert_eq!(outcome.offset, 3);
        assert!(!outcome.at_boundary);
    }

    #[test]
    fn control_scroll_uses_fast_multiplier() {
        let mut state = ScrollState::default();
        state.set_bounds(100, 10);

        let outcome = state.scroll(
            1,
            KeyModifiers {
                control: true,
                ..KeyModifiers::default()
            },
        );

        assert_eq!(outcome.offset, 15);
    }

    #[test]
    fn repeated_boundary_scroll_reports_boundary() {
        let mut state = ScrollState::default();
        state.set_bounds(4, 10);

        let outcome = state.scroll(1, KeyModifiers::default());

        assert!(outcome.at_boundary);
    }
}
