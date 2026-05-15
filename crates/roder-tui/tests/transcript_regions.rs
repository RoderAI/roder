use std::time::Instant;

use crossterm::event::{
    KeyModifiers as CrosstermKeyModifiers, MouseButton as CrosstermMouseButton, MouseEvent,
    MouseEventKind,
};
use roder_api::interactive::{InteractiveEvent, RegionKind, RegionRect};
use roder_tui::{
    mouse::{MouseRouter, RegionFrame},
    transcript::{
        LinkTarget, TranscriptFoldState, context_menu_region, link_spans, transcript_regions,
    },
};

#[test]
fn transcript_regions_emit_message_tool_and_link_targets() {
    let messages = vec![
        "user: see https://example.com".to_string(),
        "tool call: call-1 shell".to_string(),
        "assistant: open crates/roder-tui/src/app.rs:42".to_string(),
    ];

    let regions = transcript_regions(
        &messages,
        "thread",
        "turn",
        RegionRect {
            x: 0,
            y: 0,
            width: 80,
            height: 12,
        },
        &TranscriptFoldState::default(),
    );

    assert!(regions.iter().any(|region| matches!(
        region.kind,
        RegionKind::TranscriptMessage { message_idx: 0, .. }
    )));
    assert!(regions.iter().any(
        |region| matches!(region.kind, RegionKind::ToolCallBlock { ref call_id, .. } if call_id == "call-1")
    ));
    assert!(
        regions
            .iter()
            .any(|region| matches!(region.kind, RegionKind::Url(_)))
    );
    assert!(
        regions
            .iter()
            .any(|region| matches!(region.kind, RegionKind::FileReference { .. }))
    );
}

#[test]
fn transcript_linkifier_ignores_code_fence_false_positives() {
    let spans = link_spans(
        "before https://example.com\n```rust\ncrates/roder-tui/src/app.rs:12\n```\nafter",
    );

    assert_eq!(spans.len(), 1);
    assert_eq!(
        spans[0].target,
        LinkTarget::Url("https://example.com".to_string())
    );
}

#[test]
fn mouse_click_resolves_nested_link_over_message_region() {
    let messages = vec!["user: see https://example.com".to_string()];
    let regions = transcript_regions(
        &messages,
        "thread",
        "turn",
        RegionRect {
            x: 0,
            y: 0,
            width: 80,
            height: 4,
        },
        &TranscriptFoldState::default(),
    );
    let link = regions
        .iter()
        .find(|region| matches!(region.kind, RegionKind::Url(_)))
        .unwrap();
    let x = link.rect.x;
    let y = link.rect.y;
    let mut builder = RegionFrame::builder();
    for region in regions {
        builder.push(region);
    }
    let mut router = MouseRouter::new(builder.build());
    let now = Instant::now();

    router.handle_mouse_event(
        mouse(MouseEventKind::Down(CrosstermMouseButton::Left), x, y),
        now,
    );
    let events = router.handle_mouse_event(
        mouse(MouseEventKind::Up(CrosstermMouseButton::Left), x, y),
        now,
    );

    assert!(events.iter().any(|event| matches!(
        event,
        InteractiveEvent::Click { region, .. } if region.starts_with("transcript-url-")
    )));
}

#[test]
fn context_menu_region_is_clamped_to_terminal_edges() {
    let menu = context_menu_region(
        4,
        (78, 23),
        RegionRect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        },
    );

    assert_eq!(menu.area.x + menu.area.width, 80);
    assert_eq!(menu.area.y + menu.area.height, 24);
}

#[test]
fn fold_state_serializes_for_resume_storage() {
    let mut state = TranscriptFoldState::default();
    state.toggle_message(2);
    state.toggle_tool_call("call-1");

    let decoded = TranscriptFoldState::from_json_value(state.to_json_value()).unwrap();

    assert!(decoded.is_message_collapsed(2));
    assert!(!decoded.is_tool_call_expanded("call-1"));
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: CrosstermKeyModifiers::empty(),
    }
}
