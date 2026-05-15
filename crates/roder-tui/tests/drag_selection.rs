use std::time::Instant;

use crossterm::event::{
    KeyModifiers as CrosstermKeyModifiers, MouseButton as CrosstermMouseButton, MouseEvent,
    MouseEventKind,
};
use roder_api::interactive::{
    HoverCursor, InteractiveEvent, InteractiveRegion, MouseButton, RegionKind, RegionRect,
};
use roder_tui::{
    mouse::{DragSelectionOutcome, DragSelectionState, MouseRouter, RegionFrame, SelectedText},
    selection::{MemoryClipboardSink, copy_selection},
};

#[test]
fn drag_selects_transcript_text_across_message_rows() {
    let mut state = DragSelectionState::default();
    let region = transcript_region(0, 0, 0, 30, 1);
    let lines = vec!["abcd".to_string(), "efgh".to_string()];

    assert!(matches!(
        state.apply_event(
            &InteractiveEvent::DragStart {
                region: region.id.clone(),
                anchor: (1, 0),
            },
            Some(&region),
            &lines,
            ""
        ),
        Some(DragSelectionOutcome::Started(_))
    ));
    state.apply_event(
        &InteractiveEvent::DragUpdate {
            region: region.id.clone(),
            cursor: (2, 1),
        },
        Some(&region),
        &lines,
        "",
    );
    let outcome = state.apply_event(
        &InteractiveEvent::DragEnd {
            region: region.id.clone(),
            cursor: (2, 1),
        },
        Some(&region),
        &lines,
        "",
    );

    assert_eq!(
        outcome,
        Some(DragSelectionOutcome::Finalized(SelectedText::Transcript(
            "bcd\nef".to_string()
        )))
    );
}

#[test]
fn drag_selects_composer_text_by_offset() {
    let mut state = DragSelectionState::default();
    let region = composer_region(0, 0, 20, 3);

    state.apply_event(
        &InteractiveEvent::DragStart {
            region: region.id.clone(),
            anchor: (1, 0),
        },
        Some(&region),
        &[],
        "abcdef",
    );
    let outcome = state.apply_event(
        &InteractiveEvent::DragEnd {
            region: region.id.clone(),
            cursor: (4, 0),
        },
        Some(&region),
        &[],
        "abcdef",
    );

    assert_eq!(
        outcome,
        Some(DragSelectionOutcome::Finalized(SelectedText::Composer(
            "bcd".to_string()
        )))
    );
}

#[test]
fn too_short_drag_clears_selection() {
    let mut state = DragSelectionState::default().with_min_chars(3);
    let region = transcript_region(0, 0, 0, 30, 1);
    let lines = vec!["abcd".to_string()];

    state.apply_event(
        &InteractiveEvent::DragStart {
            region: region.id.clone(),
            anchor: (0, 0),
        },
        Some(&region),
        &lines,
        "",
    );
    let outcome = state.apply_event(
        &InteractiveEvent::DragEnd {
            region: region.id.clone(),
            cursor: (2, 0),
        },
        Some(&region),
        &lines,
        "",
    );

    assert_eq!(outcome, Some(DragSelectionOutcome::ClearedTooShort));
    assert!(state.transcript_range().is_none());
}

#[test]
fn hover_during_drag_does_not_clear_active_selection() {
    let mut state = DragSelectionState::default();
    let region = transcript_region(0, 0, 0, 30, 1);
    let lines = vec!["abcd".to_string()];

    state.apply_event(
        &InteractiveEvent::DragStart {
            region: region.id.clone(),
            anchor: (0, 0),
        },
        Some(&region),
        &lines,
        "",
    );
    state.apply_event(
        &InteractiveEvent::HoverEnter {
            region: "other".to_string(),
        },
        None,
        &lines,
        "",
    );

    assert!(state.transcript_range().is_some());
}

#[test]
fn short_mouse_movement_still_dispatches_click() {
    let mut builder = RegionFrame::builder();
    builder.push(transcript_region(0, 0, 0, 30, 1));
    let mut router = MouseRouter::new(builder.build()).with_drag_threshold(3);
    let now = Instant::now();

    router.handle_mouse_event(
        mouse(MouseEventKind::Down(CrosstermMouseButton::Left), 2, 0),
        now,
    );
    let events = router.handle_mouse_event(
        mouse(MouseEventKind::Up(CrosstermMouseButton::Left), 3, 0),
        now,
    );

    assert!(events.iter().any(|event| matches!(
        event,
        InteractiveEvent::Click {
            button: MouseButton::Left,
            ..
        }
    )));
}

#[test]
fn clipboard_copy_uses_injected_sink() {
    let mut sink = MemoryClipboardSink::default();

    assert!(copy_selection(&mut sink, "selected text").unwrap());

    assert_eq!(sink.writes, ["selected text"]);
}

fn transcript_region(idx: usize, x: u16, y: u16, width: u16, height: u16) -> InteractiveRegion {
    InteractiveRegion {
        id: format!("transcript-message-{idx}"),
        rect: RegionRect {
            x,
            y,
            width,
            height,
        },
        z: 0,
        kind: RegionKind::TranscriptMessage {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            message_idx: idx,
        },
        hover_cursor: HoverCursor::Pointer,
        keyboard_binding: None,
    }
}

fn composer_region(x: u16, y: u16, width: u16, height: u16) -> InteractiveRegion {
    InteractiveRegion {
        id: "composer".to_string(),
        rect: RegionRect {
            x,
            y,
            width,
            height,
        },
        z: 0,
        kind: RegionKind::Composer,
        hover_cursor: HoverCursor::Text,
        keyboard_binding: None,
    }
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: CrosstermKeyModifiers::empty(),
    }
}
