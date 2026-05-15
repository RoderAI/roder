use roder_api::interactive::HoverCursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalCursorShape {
    Default,
    Pointer,
    Text,
    Grab,
    Crosshair,
    NotAllowed,
}

#[derive(Debug, Clone)]
pub struct CursorFeedback {
    active: TerminalCursorShape,
    pointer_indicator: bool,
}

impl Default for CursorFeedback {
    fn default() -> Self {
        Self {
            active: TerminalCursorShape::Default,
            pointer_indicator: true,
        }
    }
}

impl CursorFeedback {
    pub fn update(&mut self, cursor: HoverCursor) -> TerminalCursorShape {
        self.active = terminal_shape(cursor);
        self.active
    }

    pub fn clear(&mut self) {
        self.active = TerminalCursorShape::Default;
    }

    pub fn active(&self) -> TerminalCursorShape {
        self.active
    }

    pub fn pointer_indicator(&self) -> Option<&'static str> {
        if !self.pointer_indicator {
            return None;
        }
        match self.active {
            TerminalCursorShape::Default => None,
            TerminalCursorShape::Pointer => Some("*"),
            TerminalCursorShape::Text => Some("I"),
            TerminalCursorShape::Grab => Some("G"),
            TerminalCursorShape::Crosshair => Some("+"),
            TerminalCursorShape::NotAllowed => Some("!"),
        }
    }
}

fn terminal_shape(cursor: HoverCursor) -> TerminalCursorShape {
    match cursor {
        HoverCursor::Default => TerminalCursorShape::Default,
        HoverCursor::Pointer => TerminalCursorShape::Pointer,
        HoverCursor::Text => TerminalCursorShape::Text,
        HoverCursor::Grab => TerminalCursorShape::Grab,
        HoverCursor::Crosshair => TerminalCursorShape::Crosshair,
        HoverCursor::NotAllowed => TerminalCursorShape::NotAllowed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_feedback_tracks_shape_and_indicator() {
        let mut feedback = CursorFeedback::default();

        assert_eq!(
            feedback.update(HoverCursor::Pointer),
            TerminalCursorShape::Pointer
        );
        assert_eq!(feedback.pointer_indicator(), Some("*"));

        feedback.clear();
        assert_eq!(feedback.active(), TerminalCursorShape::Default);
        assert_eq!(feedback.pointer_indicator(), None);
    }
}
