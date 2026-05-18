use roder_api::interactive::{
    HoverCursor, InteractiveEvent, InteractiveRegion, RegionKind, RegionRect,
};
use roder_tui::{
    mouse::{
        DragSelection, DragSelectionContent, DragSelectionController, RegionFrame,
        drag_selection_text,
    },
    selection::{OffsetRange, Point, Range},
};

fn region(id: &str, rect: RegionRect, kind: RegionKind) -> InteractiveRegion {
    InteractiveRegion {
        id: id.to_string(),
        rect,
        z: 0,
        kind,
        hover_cursor: HoverCursor::Text,
        keyboard_binding: None,
    }
}

#[test]
fn transcript_drag_update_finalizes_text_across_rows() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "message-1",
        RegionRect {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
        },
        RegionKind::TranscriptMessage {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            message_idx: 3,
        },
    ));
    let content =
        DragSelectionContent::new().with_transcript_lines("message-1", ["abcde", "fghij"]);
    let mut controller = DragSelectionController::new();

    assert!(controller.handle_event(
        &frame,
        &InteractiveEvent::DragStart {
            region: "message-1".to_string(),
            anchor: (2, 0),
        },
    ));
    assert!(controller.handle_event_with_content(
        &frame,
        &InteractiveEvent::DragEnd {
            region: "message-1".to_string(),
            cursor: (1, 1),
        },
        &content,
    ));

    let finalized = controller.finalized().expect("selection finalized");
    assert_eq!(drag_selection_text(finalized, &content), "cde\nfg");
}

#[test]
fn composer_drag_update_uses_wrapped_offsets_and_clears_short_drags() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "composer",
        RegionRect {
            x: 0,
            y: 0,
            width: 4,
            height: 3,
        },
        RegionKind::Composer,
    ));
    let content = DragSelectionContent::new().with_composer_text("composer", "abcdefghijkl");
    let mut controller = DragSelectionController::new();

    assert!(controller.handle_event(
        &frame,
        &InteractiveEvent::DragStart {
            region: "composer".to_string(),
            anchor: (2, 0),
        },
    ));
    assert!(controller.handle_event_with_content(
        &frame,
        &InteractiveEvent::DragEnd {
            region: "composer".to_string(),
            cursor: (1, 1),
        },
        &content,
    ));
    assert_eq!(
        controller
            .finalized()
            .map(|selection| drag_selection_text(selection, &content)),
        Some("cdef".to_string())
    );

    assert!(controller.handle_event(
        &frame,
        &InteractiveEvent::DragStart {
            region: "composer".to_string(),
            anchor: (0, 0),
        },
    ));
    assert!(controller.handle_event_with_content(
        &frame,
        &InteractiveEvent::DragEnd {
            region: "composer".to_string(),
            cursor: (1, 0),
        },
        &content,
    ));
    assert!(controller.finalized().is_none());
}

#[test]
fn transcript_drag_can_span_multiple_message_regions() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "message-1",
        RegionRect {
            x: 0,
            y: 0,
            width: 10,
            height: 1,
        },
        RegionKind::TranscriptMessage {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            message_idx: 1,
        },
    ));
    frame.push(region(
        "message-2",
        RegionRect {
            x: 0,
            y: 1,
            width: 10,
            height: 1,
        },
        RegionKind::TranscriptMessage {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            message_idx: 2,
        },
    ));
    let content = DragSelectionContent::new()
        .with_transcript_lines("message-1", ["abcde"])
        .with_transcript_lines("message-2", ["fghij"]);
    let mut controller = DragSelectionController::new();

    assert!(controller.handle_event(
        &frame,
        &InteractiveEvent::DragStart {
            region: "message-1".to_string(),
            anchor: (2, 0),
        },
    ));
    assert!(controller.handle_event_with_content(
        &frame,
        &InteractiveEvent::DragEnd {
            region: "message-2".to_string(),
            cursor: (1, 1),
        },
        &content,
    ));

    let finalized = controller.finalized().expect("selection finalized");
    assert_eq!(drag_selection_text(finalized, &content), "cde\nfg");
}

#[test]
fn hover_events_do_not_interrupt_active_drag_selection() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "message-1",
        RegionRect {
            x: 0,
            y: 0,
            width: 10,
            height: 1,
        },
        RegionKind::TranscriptMessage {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            message_idx: 1,
        },
    ));
    let mut controller = DragSelectionController::new();

    assert!(controller.handle_event(
        &frame,
        &InteractiveEvent::DragStart {
            region: "message-1".to_string(),
            anchor: (1, 0),
        },
    ));
    assert!(!controller.handle_event(
        &frame,
        &InteractiveEvent::HoverEnter {
            region: "message-1".to_string(),
        },
    ));
    assert!(controller.active().is_some());
}

#[test]
fn transcript_drag_start_begins_line_column_selection() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "message-1",
        RegionRect {
            x: 4,
            y: 8,
            width: 40,
            height: 5,
        },
        RegionKind::TranscriptMessage {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            message_idx: 3,
        },
    ));

    let mut controller = DragSelectionController::new();
    assert!(controller.handle_event(
        &frame,
        &InteractiveEvent::DragStart {
            region: "message-1".to_string(),
            anchor: (9, 10),
        },
    ));

    assert_eq!(
        controller.active(),
        Some(&DragSelection::Transcript {
            region: "message-1".to_string(),
            cursor_region: "message-1".to_string(),
            message_idx: 3,
            range: Range::new(Point { line: 2, col: 5 }, Point { line: 2, col: 5 }),
        })
    );
}

#[test]
fn composer_drag_start_begins_offset_selection() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "composer",
        RegionRect {
            x: 2,
            y: 20,
            width: 12,
            height: 3,
        },
        RegionKind::Composer,
    ));

    let mut controller = DragSelectionController::new();
    assert!(controller.handle_event(
        &frame,
        &InteractiveEvent::DragStart {
            region: "composer".to_string(),
            anchor: (5, 22),
        },
    ));

    assert_eq!(
        controller.active(),
        Some(&DragSelection::Composer {
            region: "composer".to_string(),
            range: OffsetRange::new(27, 27),
        })
    );
}
