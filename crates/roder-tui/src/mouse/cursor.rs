use roder_api::interactive::HoverCursor;

pub fn cursor_shape_escape(cursor: HoverCursor) -> Option<&'static str> {
    match cursor {
        HoverCursor::Default
        | HoverCursor::Pointer
        | HoverCursor::Grab
        | HoverCursor::NotAllowed => None,
        HoverCursor::Text => Some("\x1b[6 q"),
        HoverCursor::Crosshair => Some("\x1b[2 q"),
    }
}

pub fn pointer_indicator(cursor: HoverCursor) -> Option<&'static str> {
    match cursor {
        HoverCursor::Default => None,
        HoverCursor::Pointer => Some(">"),
        HoverCursor::Text => Some("I"),
        HoverCursor::Grab => Some("#"),
        HoverCursor::Crosshair => Some("+"),
        HoverCursor::NotAllowed => Some("!"),
    }
}

#[cfg(test)]
mod tests {
    use roder_api::interactive::HoverCursor;

    use super::*;

    #[test]
    fn cursor_feedback_prefers_escape_when_available() {
        assert_eq!(cursor_shape_escape(HoverCursor::Text), Some("\x1b[6 q"));
        assert_eq!(cursor_shape_escape(HoverCursor::Pointer), None);
        assert_eq!(pointer_indicator(HoverCursor::Pointer), Some(">"));
        assert_eq!(pointer_indicator(HoverCursor::Default), None);
    }
}
