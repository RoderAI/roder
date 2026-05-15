use roder_api::interactive::{HoverCursor, InteractiveRegion, RegionKind, RegionRect};

use crate::transcript::{
    TranscriptFoldState,
    links::{LinkTarget, link_spans},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptAction {
    ToggleToolCall { call_id: String },
    ToggleMessage { message_idx: usize },
    OpenUrl { url: String },
    OpenFile { path: String, line: Option<u32> },
    OpenContextMenu { message_idx: usize, at: (u16, u16) },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptContextMenu {
    pub message_idx: usize,
    pub area: RegionRect,
}

pub fn transcript_regions(
    messages: &[String],
    thread_id: &str,
    turn_id: &str,
    area: RegionRect,
    fold_state: &TranscriptFoldState,
) -> Vec<InteractiveRegion> {
    let mut regions = Vec::new();
    for (idx, message) in messages.iter().enumerate() {
        let y = area.y.saturating_add((idx as u16).saturating_mul(2));
        if y >= area.y.saturating_add(area.height) {
            break;
        }
        let row = RegionRect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        };
        regions.push(InteractiveRegion {
            id: message_region_id(idx),
            rect: row,
            z: 0,
            kind: RegionKind::TranscriptMessage {
                thread_id: thread_id.to_string(),
                turn_id: turn_id.to_string(),
                message_idx: idx,
            },
            hover_cursor: HoverCursor::Pointer,
            keyboard_binding: None,
        });

        if let Some(call_id) = tool_call_id(message) {
            regions.push(InteractiveRegion {
                id: tool_call_region_id(&call_id),
                rect: row,
                z: 1,
                kind: RegionKind::ToolCallBlock {
                    expanded: fold_state.is_tool_call_expanded(&call_id),
                    call_id,
                },
                hover_cursor: HoverCursor::Pointer,
                keyboard_binding: None,
            });
        }

        let (link_text, body_offset) = link_text_and_render_offset(message);
        for (link_idx, span) in link_spans(link_text).into_iter().enumerate() {
            let span_start = body_offset.saturating_add(span.start) as u16;
            let span_width = span.end.saturating_sub(span.start).max(1) as u16;
            let rect = RegionRect {
                x: area
                    .x
                    .saturating_add(span_start)
                    .min(area.x.saturating_add(area.width)),
                y,
                width: span_width.min(area.width),
                height: 1,
            };
            let (id, kind) = match span.target {
                LinkTarget::Url(url) => (url_region_id(idx, link_idx), RegionKind::Url(url)),
                LinkTarget::File { path, line } => (
                    file_region_id(idx, link_idx),
                    RegionKind::FileReference { path, line },
                ),
            };
            regions.push(InteractiveRegion {
                id,
                rect,
                z: 2,
                kind,
                hover_cursor: HoverCursor::Pointer,
                keyboard_binding: None,
            });
        }
    }
    regions
}

pub fn action_for_region(
    region: &InteractiveRegion,
    right_click_at: Option<(u16, u16)>,
) -> Option<TranscriptAction> {
    match &region.kind {
        RegionKind::ToolCallBlock { call_id, .. } => Some(TranscriptAction::ToggleToolCall {
            call_id: call_id.clone(),
        }),
        RegionKind::TranscriptMessage { message_idx, .. } => right_click_at
            .map(|at| TranscriptAction::OpenContextMenu {
                message_idx: *message_idx,
                at,
            })
            .or(Some(TranscriptAction::ToggleMessage {
                message_idx: *message_idx,
            })),
        RegionKind::Url(url) => Some(TranscriptAction::OpenUrl { url: url.clone() }),
        RegionKind::FileReference { path, line } => Some(TranscriptAction::OpenFile {
            path: path.display().to_string(),
            line: *line,
        }),
        _ => None,
    }
}

pub fn context_menu_region(
    message_idx: usize,
    at: (u16, u16),
    terminal: RegionRect,
) -> TranscriptContextMenu {
    let width = 22u16.min(terminal.width);
    let height = 3u16.min(terminal.height);
    let max_x = terminal
        .x
        .saturating_add(terminal.width.saturating_sub(width));
    let max_y = terminal
        .y
        .saturating_add(terminal.height.saturating_sub(height));
    TranscriptContextMenu {
        message_idx,
        area: RegionRect {
            x: at.0.clamp(terminal.x, max_x),
            y: at.1.clamp(terminal.y, max_y),
            width,
            height,
        },
    }
}

fn link_text_and_render_offset(message: &str) -> (&str, usize) {
    if let Some(body) = message.strip_prefix("user: ") {
        (body, 4)
    } else if let Some(body) = message.strip_prefix("assistant: ") {
        (body, 8)
    } else if let Some(body) = message.strip_prefix("error: ") {
        (body, 2)
    } else if let Some(body) = message.strip_prefix("system: ") {
        (body, 2)
    } else if let Some(body) = message.strip_prefix("shell output: ") {
        (body, 2)
    } else if let Some(body) = message.strip_prefix("shell: ") {
        (body, 2)
    } else if let Some(body) = message.strip_prefix("tool call: ") {
        (body, 5)
    } else {
        (message, 0)
    }
}

fn tool_call_id(message: &str) -> Option<String> {
    let body = message.strip_prefix("tool call: ")?;
    body.split_whitespace().next().map(str::to_string)
}

fn message_region_id(idx: usize) -> String {
    format!("transcript-message-{idx}")
}

fn tool_call_region_id(call_id: &str) -> String {
    format!("tool-call-{call_id}")
}

fn url_region_id(message_idx: usize, link_idx: usize) -> String {
    format!("transcript-url-{message_idx}-{link_idx}")
}

fn file_region_id(message_idx: usize, link_idx: usize) -> String {
    format!("transcript-file-{message_idx}-{link_idx}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(regions.iter().any(|region| matches!(region.kind, RegionKind::ToolCallBlock { ref call_id, .. } if call_id == "call-1")));
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
    fn context_menu_stays_inside_terminal_edges() {
        let menu = context_menu_region(
            1,
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
}
