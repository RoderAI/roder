use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use roder_api::interactive::{
    HandlerOutcome, HoverCursor, InteractiveEvent, InteractiveMouseButton, InteractiveRegion,
    InteractiveRegionHandler, RegionKind, RegionRect,
};
use roder_tui::mouse::{MouseRouter, RegionFrame, RegionHandlerDispatcher};

fn region(id: &str, rect: RegionRect, z: i16) -> InteractiveRegion {
    InteractiveRegion {
        id: id.to_string(),
        rect,
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
        modifiers: KeyModifiers::NONE,
    }
}

fn mouse_with_modifiers(
    kind: MouseEventKind,
    column: u16,
    row: u16,
    modifiers: KeyModifiers,
) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers,
    }
}

#[test]
fn hit_testing_prefers_topmost_and_latest_region() {
    let mut frame = RegionFrame::new();
    let rect = RegionRect {
        x: 1,
        y: 1,
        width: 5,
        height: 5,
    };
    frame.push(region("low", rect, 0));
    frame.push(region("top-old", rect, 3));
    frame.push(region("top-new", rect, 3));

    assert_eq!(
        frame.hit_test(2, 2).map(|region| region.id.as_str()),
        Some("top-new")
    );
}

#[test]
fn hover_events_only_fire_when_region_changes() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "a",
        RegionRect {
            x: 0,
            y: 0,
            width: 5,
            height: 5,
        },
        0,
    ));
    frame.push(region(
        "b",
        RegionRect {
            x: 10,
            y: 0,
            width: 5,
            height: 5,
        },
        0,
    ));
    let mut router = MouseRouter::new();

    assert_eq!(
        router.process(&frame, mouse(MouseEventKind::Moved, 1, 1)),
        vec![InteractiveEvent::HoverEnter {
            region: "a".to_string()
        }]
    );
    assert!(
        router
            .process(&frame, mouse(MouseEventKind::Moved, 2, 2))
            .is_empty()
    );
    assert_eq!(
        router.process(&frame, mouse(MouseEventKind::Moved, 11, 1)),
        vec![
            InteractiveEvent::HoverLeave {
                region: "a".to_string()
            },
            InteractiveEvent::HoverEnter {
                region: "b".to_string()
            }
        ]
    );
}

#[test]
fn click_and_double_click_are_deterministic() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "button",
        RegionRect {
            x: 0,
            y: 0,
            width: 5,
            height: 5,
        },
        0,
    ));
    let mut router = MouseRouter::new();
    let start = Instant::now();

    router.process_at(
        &frame,
        mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
        start,
    );
    assert_eq!(
        router.process_at(
            &frame,
            mouse(MouseEventKind::Up(MouseButton::Left), 1, 1),
            start + Duration::from_millis(10),
        ),
        vec![InteractiveEvent::Click {
            region: "button".to_string(),
            modifiers: Default::default(),
            button: InteractiveMouseButton::Left,
        }]
    );
    router.process_at(
        &frame,
        mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
        start + Duration::from_millis(100),
    );
    assert_eq!(
        router.process_at(
            &frame,
            mouse(MouseEventKind::Up(MouseButton::Left), 1, 1),
            start + Duration::from_millis(120),
        ),
        vec![InteractiveEvent::DoubleClick {
            region: "button".to_string(),
            modifiers: Default::default(),
        }]
    );
}

#[test]
fn drag_starts_after_threshold_and_ends_on_mouse_up() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "row",
        RegionRect {
            x: 0,
            y: 0,
            width: 20,
            height: 5,
        },
        0,
    ));
    let mut router = MouseRouter::new();

    router.process(&frame, mouse(MouseEventKind::Down(MouseButton::Left), 1, 1));
    assert!(
        router
            .process(&frame, mouse(MouseEventKind::Drag(MouseButton::Left), 2, 1))
            .is_empty()
    );
    assert_eq!(
        router.process(&frame, mouse(MouseEventKind::Drag(MouseButton::Left), 5, 1)),
        vec![
            InteractiveEvent::DragStart {
                region: "row".to_string(),
                anchor: (1, 1),
            },
            InteractiveEvent::DragUpdate {
                region: "row".to_string(),
                cursor: (5, 1),
            }
        ]
    );
    assert_eq!(
        router.process(&frame, mouse(MouseEventKind::Up(MouseButton::Left), 6, 1)),
        vec![InteractiveEvent::DragEnd {
            region: "row".to_string(),
            cursor: (6, 1),
        }]
    );
}

#[test]
fn short_drag_below_threshold_dispatches_click_on_mouse_up() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "row",
        RegionRect {
            x: 0,
            y: 0,
            width: 20,
            height: 5,
        },
        0,
    ));
    let mut router = MouseRouter::new();

    router.process(&frame, mouse(MouseEventKind::Down(MouseButton::Left), 1, 1));
    assert!(
        router
            .process(&frame, mouse(MouseEventKind::Drag(MouseButton::Left), 2, 1))
            .is_empty()
    );
    assert_eq!(
        router.process(&frame, mouse(MouseEventKind::Up(MouseButton::Left), 2, 1)),
        vec![InteractiveEvent::Click {
            region: "row".to_string(),
            modifiers: Default::default(),
            button: InteractiveMouseButton::Left,
        }]
    );
}

#[test]
fn scroll_targets_region_under_cursor() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "timeline",
        RegionRect {
            x: 0,
            y: 0,
            width: 20,
            height: 5,
        },
        0,
    ));
    let mut router = MouseRouter::new();

    assert_eq!(
        router.process(&frame, mouse(MouseEventKind::ScrollDown, 1, 1)),
        vec![InteractiveEvent::Scroll {
            region: Some("timeline".to_string()),
            delta_lines: 3,
            modifiers: Default::default(),
        }]
    );
}

#[test]
fn scroll_honors_lines_per_tick_and_control_fast_scroll() {
    let mut frame = RegionFrame::new();
    frame.push(region(
        "timeline",
        RegionRect {
            x: 0,
            y: 0,
            width: 20,
            height: 5,
        },
        0,
    ));
    let mut router = MouseRouter::new().with_scroll_lines_per_tick(4);

    assert_eq!(
        router.process(&frame, mouse(MouseEventKind::ScrollDown, 1, 1)),
        vec![InteractiveEvent::Scroll {
            region: Some("timeline".to_string()),
            delta_lines: 4,
            modifiers: Default::default(),
        }]
    );
    assert_eq!(
        router.process(
            &frame,
            mouse_with_modifiers(MouseEventKind::ScrollUp, 1, 1, KeyModifiers::CONTROL)
        ),
        vec![InteractiveEvent::Scroll {
            region: Some("timeline".to_string()),
            delta_lines: -20,
            modifiers: roder_api::interactive::InteractiveModifiers {
                control: true,
                ..Default::default()
            },
        }]
    );
}

struct ConsumingComposerHandler {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl InteractiveRegionHandler for ConsumingComposerHandler {
    fn id(&self) -> String {
        "composer-test-handler".to_string()
    }

    fn kinds(&self) -> &[&'static str] {
        &["Composer"]
    }

    async fn handle(
        &self,
        _event: InteractiveEvent,
        _region: &InteractiveRegion,
    ) -> anyhow::Result<HandlerOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(HandlerOutcome::Consumed)
    }
}

#[tokio::test]
async fn mouse_integration_router_dispatches_region_events_to_registered_handlers() {
    let calls = Arc::new(AtomicUsize::new(0));
    let dispatcher = RegionHandlerDispatcher::new(vec![Arc::new(ConsumingComposerHandler {
        calls: Arc::clone(&calls),
    })]);
    let mut frame = RegionFrame::new();
    frame.push(region(
        "composer",
        RegionRect {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
        },
        0,
    ));
    let mut router = MouseRouter::new();

    let routed = router
        .process_and_dispatch(&frame, &dispatcher, mouse(MouseEventKind::Moved, 1, 1))
        .await
        .unwrap();

    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0].region.as_deref(), Some("composer"));
    assert_eq!(routed[0].outcome, HandlerOutcome::Consumed);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
