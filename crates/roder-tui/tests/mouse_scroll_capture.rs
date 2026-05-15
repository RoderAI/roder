use crossterm::event::{KeyModifiers as CrosstermKeyModifiers, MouseEvent, MouseEventKind};
use roder_api::interactive::{InteractiveEvent, KeyModifiers, RegionRect};
use roder_tui::mouse::{
    MouseCaptureController, MouseCaptureEvent, MouseRouter, RegionFrame, ScrollState,
};

#[test]
fn scroll_state_moves_inside_scroll_target() {
    let mut state = ScrollState::default();
    state.set_bounds(40, 10);

    let outcome = state.scroll(1, KeyModifiers::default());

    assert_eq!(outcome.offset, 3);
    assert!(!outcome.at_boundary);
}

#[test]
fn scroll_state_honors_control_fast_scroll() {
    let mut state = ScrollState::default();
    state.set_bounds(80, 10);

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
fn boundary_scroll_releases_capture_until_window_expires() {
    let mut scroll = ScrollState::default();
    scroll.set_bounds(3, 10);
    let mut capture = MouseCaptureController::default();

    let outcome = scroll.scroll(1, KeyModifiers::default());
    assert!(outcome.at_boundary);
    assert_eq!(
        capture.release_for_boundary_scroll(4),
        Some(MouseCaptureEvent::CaptureDisabled)
    );
    assert_eq!(capture.tick(8), None);
    assert_eq!(capture.tick(12), Some(MouseCaptureEvent::CaptureEnabled));
}

#[test]
fn router_emits_scroll_event_with_region_and_modifiers() {
    let mut builder = RegionFrame::builder();
    builder.push(roder_api::interactive::InteractiveRegion {
        id: "transcript-message-0".to_string(),
        rect: RegionRect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        },
        z: 0,
        kind: roder_api::interactive::RegionKind::TranscriptMessage {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            message_idx: 0,
        },
        hover_cursor: roder_api::interactive::HoverCursor::Pointer,
        keyboard_binding: None,
    });
    let mut router = MouseRouter::new(builder.build());
    let event = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 2,
        row: 2,
        modifiers: CrosstermKeyModifiers::CONTROL,
    };

    let events = router.handle_mouse_event(event, std::time::Instant::now());

    assert!(events.iter().any(|event| matches!(
        event,
        InteractiveEvent::Scroll {
            region: Some(region),
            delta_lines: 1,
            modifiers,
        } if region == "transcript-message-0" && modifiers.control
    )));
}
