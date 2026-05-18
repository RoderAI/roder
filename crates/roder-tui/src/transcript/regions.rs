use roder_api::events::{ThreadId, TurnId};
use roder_api::interactive::{HoverCursor, InteractiveRegion, RegionId, RegionKind, RegionRect};
use std::path::PathBuf;

pub fn transcript_message_region(
    id: RegionId,
    rect: RegionRect,
    z: i16,
    thread_id: ThreadId,
    turn_id: TurnId,
    message_idx: usize,
) -> InteractiveRegion {
    InteractiveRegion {
        id,
        rect,
        z,
        kind: RegionKind::TranscriptMessage {
            thread_id,
            turn_id,
            message_idx,
        },
        hover_cursor: HoverCursor::Text,
        keyboard_binding: None,
    }
}

pub fn tool_call_region(
    id: RegionId,
    rect: RegionRect,
    z: i16,
    call_id: String,
    expanded: bool,
) -> InteractiveRegion {
    InteractiveRegion {
        id,
        rect,
        z,
        kind: RegionKind::ToolCallBlock { call_id, expanded },
        hover_cursor: HoverCursor::Pointer,
        keyboard_binding: None,
    }
}

pub fn url_region(id: RegionId, rect: RegionRect, z: i16, url: String) -> InteractiveRegion {
    InteractiveRegion {
        id,
        rect,
        z,
        kind: RegionKind::Url(url),
        hover_cursor: HoverCursor::Pointer,
        keyboard_binding: None,
    }
}

pub fn file_reference_region(
    id: RegionId,
    rect: RegionRect,
    z: i16,
    path: PathBuf,
    line: Option<u32>,
) -> InteractiveRegion {
    InteractiveRegion {
        id,
        rect,
        z,
        kind: RegionKind::FileReference { path, line },
        hover_cursor: HoverCursor::Pointer,
        keyboard_binding: None,
    }
}
