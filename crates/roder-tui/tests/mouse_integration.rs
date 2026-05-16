use std::time::Instant;

use crossterm::event::{
    KeyModifiers as CrosstermKeyModifiers, MouseButton as CrosstermMouseButton, MouseEvent,
    MouseEventKind,
};
use roder_api::interactive::{
    HandlerOutcome, InteractiveEvent, InteractiveRegion, InteractiveRegionHandler,
    InteractiveRegionHandlerId, KeyModifiers, MouseButton, RegionKind, RegionRect,
};
use roder_tui::{
    mouse::{
        DragSelectionOutcome, DragSelectionState, MouseRouter, RegionFrame, ScrollState,
        SelectedText,
    },
    selection::{MemoryClipboardSink, copy_selection},
    transcript::{
        TranscriptAction, TranscriptFoldState, action_for_region, transcript_regions,
    },
};

struct PassthroughUrlHandler;

#[async_trait::async_trait]
impl InteractiveRegionHandler for PassthroughUrlHandler {
    fn id(&self) -> InteractiveRegionHandlerId {
        "test-url-handler".to_string()
    }

    fn kinds(&self) -> Vec<String> {
        vec!["Url".to_string()]
    }

    async fn handle(
        &self,
        _event: InteractiveEvent,
        _region: &InteractiveRegion,
    ) -> anyhow::Result<HandlerOutcome> {
        Ok(HandlerOutcome::Passthrough)
    }
}

#[tokio::test]
async fn mouse_integration_exercises_transcript_regions_and_extension_handler() {
    let messages = vec![
        "user: see https://example.com before running tools".to_string(),
        "tool call: call-1 shell".to_string(),
        "assistant: open crates/roder-tui/src/app.rs:42".to_string(),
    ];
    let mut fold_state = TranscriptFoldState::default();
    let regions = transcript_regions(
        &messages,
        "thread",
        "turn",
        RegionRect {
            x: 0,
            y: 0,
            width: 90,
            height: 12,
        },
        &fold_state,
    );
    let mut builder = RegionFrame::builder();
    for region in regions.clone() {
        builder.push(region);
    }
    let frame = builder.build();
    let mut router = MouseRouter::new(frame.clone());

    let url = regions
        .iter()
        .find(|region| matches!(region.kind, RegionKind::Url(_)))
        .expect("url region");
    let hover = router.handle_mouse_event(
        mouse(MouseEventKind::Moved, url.rect.x, url.rect.y),
        Instant::now(),
    );
    assert!(matches!(
        hover.as_slice(),
        [InteractiveEvent::HoverEnter { region }] if region.starts_with("transcript-url-")
    ));

    let tool = regions
        .iter()
        .find(|region| matches!(region.kind, RegionKind::ToolCallBlock { .. }))
        .expect("tool call region");
    router.handle_mouse_event(
        mouse(
            MouseEventKind::Down(CrosstermMouseButton::Left),
            tool.rect.x,
            tool.rect.y,
        ),
        Instant::now(),
    );
    let click = router.handle_mouse_event(
        mouse(
            MouseEventKind::Up(CrosstermMouseButton::Left),
            tool.rect.x,
            tool.rect.y,
        ),
        Instant::now(),
    );
    let clicked_region = click
        .iter()
        .find_map(|event| match event {
            InteractiveEvent::Click { region, .. } => Some(region.as_str()),
            _ => None,
        })
        .expect("click event");
    let clicked = frame.get(clicked_region).expect("clicked region");
    assert_eq!(
        action_for_region(clicked, None),
        Some(TranscriptAction::ToggleToolCall {
            call_id: "call-1".to_string()
        })
    );
    fold_state.toggle_tool_call("call-1");
    assert!(!fold_state.is_tool_call_expanded("call-1"));

    let message = regions
        .iter()
        .find(|region| matches!(region.kind, RegionKind::TranscriptMessage { message_idx: 0, .. }))
        .expect("message region");
    let mut drag = DragSelectionState::default().with_min_chars(3);
    drag.apply_event(
        &InteractiveEvent::DragStart {
            region: message.id.clone(),
            anchor: (message.rect.x, message.rect.y),
        },
        Some(message),
        &messages,
        "",
    );
    let outcome = drag.apply_event(
        &InteractiveEvent::DragEnd {
            region: message.id.clone(),
            cursor: (message.rect.x + 9, message.rect.y),
        },
        Some(message),
        &messages,
        "",
    );
    let Some(DragSelectionOutcome::Finalized(SelectedText::Transcript(text))) = outcome else {
        panic!("drag did not finalize transcript selection: {outcome:?}");
    };
    let mut clipboard = MemoryClipboardSink::default();
    assert!(copy_selection(&mut clipboard, &text).unwrap());
    assert_eq!(clipboard.writes, vec![text]);

    let mut scroll = ScrollState::default();
    scroll.set_bounds(40, 10);
    let scrolled = scroll.scroll(1, KeyModifiers::default());
    assert_eq!(scrolled.offset, 3);
    assert!(!scrolled.at_boundary);

    let handler = PassthroughUrlHandler;
    assert!(handler.kinds().iter().any(|kind| kind == "Url"));
    let handled = handler
        .handle(
            InteractiveEvent::Click {
                region: url.id.clone(),
                modifiers: KeyModifiers::default(),
                button: MouseButton::Left,
            },
            url,
        )
        .await
        .unwrap();
    assert_eq!(handled, HandlerOutcome::Passthrough);
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: CrosstermKeyModifiers::empty(),
    }
}
