use std::time::{Duration, Instant};

use crossterm::event::{
    KeyModifiers as CrosstermKeyModifiers, MouseButton as CrosstermMouseButton, MouseEvent,
    MouseEventKind,
};
use roder_api::interactive::{
    HoverCursor, InteractiveEvent, InteractiveRegion, RegionKind, RegionRect,
};
use roder_tui::mouse::{MouseRouter, RegionFrame};

#[test]
fn mouse_router_hit_tests_overlapping_regions() {
    let router = MouseRouter::new(overlapping_frame());
    let frame = router_frame_hit_test_fixture();

    assert_eq!(frame.hit_test(2, 2).unwrap().id, "top");
    assert_eq!(frame.hit_test(8, 8).unwrap().id, "base");
    assert!(frame.hit_test(20, 20).is_none());
    drop(router);
}

#[test]
fn mouse_router_suppresses_redundant_hover_events() {
    let mut router = MouseRouter::new(overlapping_frame());
    let now = Instant::now();

    assert_eq!(
        router.handle_mouse_event(mouse(MouseEventKind::Moved, 8, 8), now),
        vec![InteractiveEvent::HoverEnter {
            region: "base".to_string()
        }]
    );
    assert!(
        router
            .handle_mouse_event(mouse(MouseEventKind::Moved, 9, 9), now)
            .is_empty()
    );
    assert_eq!(
        router.handle_mouse_event(mouse(MouseEventKind::Moved, 20, 20), now),
        vec![InteractiveEvent::HoverLeave {
            region: "base".to_string()
        }]
    );
}

#[test]
fn mouse_router_detects_double_clicks() {
    let mut router =
        MouseRouter::new(overlapping_frame()).with_double_click_window(Duration::from_millis(350));
    let t0 = Instant::now();
    router.handle_mouse_event(
        mouse(MouseEventKind::Down(CrosstermMouseButton::Left), 8, 8),
        t0,
    );
    let first = router.handle_mouse_event(
        mouse(MouseEventKind::Up(CrosstermMouseButton::Left), 8, 8),
        t0,
    );
    router.handle_mouse_event(
        mouse(MouseEventKind::Down(CrosstermMouseButton::Left), 8, 8),
        t0 + Duration::from_millis(100),
    );
    let second = router.handle_mouse_event(
        mouse(MouseEventKind::Up(CrosstermMouseButton::Left), 8, 8),
        t0 + Duration::from_millis(100),
    );

    assert!(matches!(first.as_slice(), [InteractiveEvent::Click { .. }]));
    assert!(matches!(
        second.as_slice(),
        [
            InteractiveEvent::Click { .. },
            InteractiveEvent::DoubleClick { .. }
        ]
    ));
}

#[test]
fn mouse_router_turns_threshold_crossing_into_drag() {
    let mut router = MouseRouter::new(overlapping_frame()).with_drag_threshold(3);
    let now = Instant::now();

    router.handle_mouse_event(
        mouse(MouseEventKind::Down(CrosstermMouseButton::Left), 8, 8),
        now,
    );
    assert!(
        router
            .handle_mouse_event(
                mouse(MouseEventKind::Drag(CrosstermMouseButton::Left), 9, 8),
                now
            )
            .is_empty()
    );
    assert_eq!(
        router.handle_mouse_event(
            mouse(MouseEventKind::Drag(CrosstermMouseButton::Left), 11, 8),
            now
        ),
        vec![InteractiveEvent::DragStart {
            region: "base".to_string(),
            anchor: (8, 8),
        }]
    );
    assert_eq!(
        router.handle_mouse_event(
            mouse(MouseEventKind::Up(CrosstermMouseButton::Left), 12, 8),
            now
        ),
        vec![InteractiveEvent::DragEnd {
            region: "base".to_string(),
            cursor: (12, 8),
        }]
    );
}

fn overlapping_frame() -> RegionFrame {
    let mut builder = RegionFrame::builder();
    builder.push(region("base", 0, 0, 0, 10, 10));
    builder.push(region("top", 1, 0, 0, 5, 5));
    builder.build()
}

fn router_frame_hit_test_fixture() -> RegionFrame {
    overlapping_frame()
}

fn region(id: &str, z: i16, x: u16, y: u16, width: u16, height: u16) -> InteractiveRegion {
    InteractiveRegion {
        id: id.to_string(),
        rect: RegionRect {
            x,
            y,
            width,
            height,
        },
        z,
        kind: RegionKind::Composer,
        hover_cursor: HoverCursor::Pointer,
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
