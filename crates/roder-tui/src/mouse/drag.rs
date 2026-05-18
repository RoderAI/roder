use std::collections::BTreeMap;

use roder_api::interactive::{InteractiveEvent, RegionId, RegionKind};

use crate::selection::{OffsetRange, Point, Range, selected_offset_text, selected_text};

use super::regions::RegionFrame;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragSelection {
    Transcript {
        region: RegionId,
        cursor_region: RegionId,
        message_idx: usize,
        range: Range,
    },
    Composer {
        region: RegionId,
        range: OffsetRange,
    },
}

#[derive(Debug, Clone)]
pub struct DragSelectionController {
    active: Option<DragSelection>,
    finalized: Option<DragSelection>,
    min_chars: usize,
}

impl Default for DragSelectionController {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct DragSelectionContent {
    pub transcript_lines: BTreeMap<RegionId, Vec<String>>,
    pub composer_text: BTreeMap<RegionId, String>,
}

impl Default for DragSelectionContent {
    fn default() -> Self {
        Self::new()
    }
}

impl DragSelectionContent {
    pub fn new() -> Self {
        Self {
            transcript_lines: BTreeMap::new(),
            composer_text: BTreeMap::new(),
        }
    }

    pub fn with_transcript_lines(
        mut self,
        region: impl Into<RegionId>,
        lines: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let region = region.into();
        self.transcript_lines
            .insert(region, lines.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_composer_text(
        mut self,
        region: impl Into<RegionId>,
        text: impl Into<String>,
    ) -> Self {
        self.composer_text.insert(region.into(), text.into());
        self
    }
}

impl DragSelectionController {
    pub fn new() -> Self {
        Self {
            active: None,
            finalized: None,
            min_chars: 3,
        }
    }

    pub fn with_min_chars(mut self, min_chars: usize) -> Self {
        self.min_chars = min_chars;
        self
    }

    pub fn active(&self) -> Option<&DragSelection> {
        self.active.as_ref()
    }

    pub fn finalized(&self) -> Option<&DragSelection> {
        self.finalized.as_ref()
    }

    pub fn clear(&mut self) {
        self.active = None;
        self.finalized = None;
    }

    pub fn handle_event(&mut self, frame: &RegionFrame, event: &InteractiveEvent) -> bool {
        self.handle_event_with_content(frame, event, &DragSelectionContent::new())
    }

    pub fn handle_event_with_content(
        &mut self,
        frame: &RegionFrame,
        event: &InteractiveEvent,
        content: &DragSelectionContent,
    ) -> bool {
        match event {
            InteractiveEvent::DragStart { region, anchor } => {
                self.begin_selection(frame, region, *anchor)
            }
            InteractiveEvent::DragUpdate { region, cursor } => {
                self.update_selection(frame, region, *cursor)
            }
            InteractiveEvent::DragEnd { region, cursor } => {
                if !self.update_selection(frame, region, *cursor) {
                    return false;
                }
                self.finish_selection(content);
                true
            }
            _ => false,
        }
    }

    fn begin_selection(
        &mut self,
        frame: &RegionFrame,
        region_id: &RegionId,
        anchor: (u16, u16),
    ) -> bool {
        let Some(region) = frame.get(region_id) else {
            return false;
        };
        self.finalized = None;

        match &region.kind {
            RegionKind::TranscriptMessage { message_idx, .. } => {
                let point = region_point(region.rect, anchor);
                self.active = Some(DragSelection::Transcript {
                    region: region_id.clone(),
                    cursor_region: region_id.clone(),
                    message_idx: *message_idx,
                    range: Range::new(point, point),
                });
                true
            }
            RegionKind::Composer => {
                let offset = region_offset(region.rect, anchor);
                self.active = Some(DragSelection::Composer {
                    region: region_id.clone(),
                    range: OffsetRange::new(offset, offset),
                });
                true
            }
            _ => false,
        }
    }

    fn update_selection(
        &mut self,
        frame: &RegionFrame,
        region_id: &RegionId,
        cursor: (u16, u16),
    ) -> bool {
        let Some(region) = frame.get(region_id) else {
            return false;
        };
        let Some(active) = self.active.as_mut() else {
            return false;
        };

        match active {
            DragSelection::Transcript {
                cursor_region,
                range,
                ..
            } => {
                if !matches!(region.kind, RegionKind::TranscriptMessage { .. }) {
                    return false;
                }
                range.cursor = region_point(region.rect, cursor);
                *cursor_region = region_id.clone();
                true
            }
            DragSelection::Composer {
                region: active_region,
                range,
            } if active_region == region_id => {
                range.cursor = region_offset(region.rect, cursor);
                true
            }
            _ => false,
        }
    }

    fn finish_selection(&mut self, content: &DragSelectionContent) {
        let Some(selection) = self.active.take() else {
            return;
        };
        if selection_char_count(&selection, content) >= self.min_chars {
            self.finalized = Some(selection);
        } else {
            self.finalized = None;
        }
    }
}

fn selection_char_count(selection: &DragSelection, content: &DragSelectionContent) -> usize {
    drag_selection_text(selection, content)
        .chars()
        .filter(|ch| *ch != '\n')
        .count()
}

pub fn drag_selection_text(selection: &DragSelection, content: &DragSelectionContent) -> String {
    match selection {
        DragSelection::Transcript {
            region,
            cursor_region,
            range,
            ..
        } => selected_transcript_text(content, region, cursor_region, *range),
        DragSelection::Composer { region, range } => content
            .composer_text
            .get(region)
            .map(|text| selected_offset_text(text, *range))
            .unwrap_or_default(),
    }
}

fn selected_transcript_text(
    content: &DragSelectionContent,
    anchor_region: &RegionId,
    cursor_region: &RegionId,
    range: Range,
) -> String {
    if anchor_region == cursor_region {
        return content
            .transcript_lines
            .get(anchor_region)
            .map(|lines| selected_text(lines, range))
            .unwrap_or_default();
    }

    let ordered = content
        .transcript_lines
        .keys()
        .cloned()
        .collect::<Vec<RegionId>>();
    let Some(anchor_index) = ordered.iter().position(|region| region == anchor_region) else {
        return String::new();
    };
    let Some(cursor_index) = ordered.iter().position(|region| region == cursor_region) else {
        return String::new();
    };

    let (start_index, end_index, start_point, end_point) = if anchor_index <= cursor_index {
        (anchor_index, cursor_index, range.anchor, range.cursor)
    } else {
        (cursor_index, anchor_index, range.cursor, range.anchor)
    };

    let mut selected = Vec::new();
    for region_id in &ordered[start_index..=end_index] {
        let Some(lines) = content.transcript_lines.get(region_id) else {
            continue;
        };
        let local_range = if region_id == &ordered[start_index] {
            Range::new(
                start_point,
                Point {
                    line: lines.len().saturating_sub(1),
                    col: usize::MAX,
                },
            )
        } else if region_id == &ordered[end_index] {
            Range::new(Point { line: 0, col: 0 }, end_point)
        } else {
            Range::new(
                Point { line: 0, col: 0 },
                Point {
                    line: lines.len().saturating_sub(1),
                    col: usize::MAX,
                },
            )
        };
        let text = selected_text(lines, local_range);
        if !text.is_empty() {
            selected.push(text);
        }
    }
    selected.join("\n")
}

fn region_point(rect: roder_api::interactive::RegionRect, position: (u16, u16)) -> Point {
    Point {
        line: usize::from(position.1.saturating_sub(rect.y)),
        col: usize::from(position.0.saturating_sub(rect.x)),
    }
}

fn region_offset(rect: roder_api::interactive::RegionRect, position: (u16, u16)) -> usize {
    let row = position.1.saturating_sub(rect.y);
    let col = position.0.saturating_sub(rect.x);
    usize::from(row) * usize::from(rect.width.max(1)) + usize::from(col)
}

#[cfg(test)]
mod tests {
    use roder_api::interactive::{HoverCursor, InteractiveRegion, RegionRect};

    use super::*;

    fn frame_with(region: InteractiveRegion) -> RegionFrame {
        let mut frame = RegionFrame::new();
        frame.push(region);
        frame
    }

    #[test]
    fn drag_start_over_transcript_message_begins_transcript_selection() {
        let frame = frame_with(InteractiveRegion {
            id: "message-1".to_string(),
            rect: RegionRect {
                x: 2,
                y: 4,
                width: 20,
                height: 3,
            },
            z: 0,
            kind: RegionKind::TranscriptMessage {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                message_idx: 7,
            },
            hover_cursor: HoverCursor::Text,
            keyboard_binding: None,
        });
        let mut drag = DragSelectionController::new();

        assert!(drag.handle_event(
            &frame,
            &InteractiveEvent::DragStart {
                region: "message-1".to_string(),
                anchor: (5, 6),
            }
        ));

        assert_eq!(
            drag.active(),
            Some(&DragSelection::Transcript {
                region: "message-1".to_string(),
                cursor_region: "message-1".to_string(),
                message_idx: 7,
                range: Range::new(Point { line: 2, col: 3 }, Point { line: 2, col: 3 }),
            })
        );
    }

    #[test]
    fn drag_start_over_composer_begins_offset_selection() {
        let frame = frame_with(InteractiveRegion {
            id: "composer".to_string(),
            rect: RegionRect {
                x: 10,
                y: 20,
                width: 8,
                height: 2,
            },
            z: 0,
            kind: RegionKind::Composer,
            hover_cursor: HoverCursor::Text,
            keyboard_binding: None,
        });
        let mut drag = DragSelectionController::new();

        assert!(drag.handle_event(
            &frame,
            &InteractiveEvent::DragStart {
                region: "composer".to_string(),
                anchor: (13, 21),
            }
        ));

        assert_eq!(
            drag.active(),
            Some(&DragSelection::Composer {
                region: "composer".to_string(),
                range: OffsetRange::new(11, 11),
            })
        );
    }

    #[test]
    fn drag_update_and_end_finalize_copyable_transcript_selection() {
        let frame = frame_with(InteractiveRegion {
            id: "message-1".to_string(),
            rect: RegionRect {
                x: 0,
                y: 0,
                width: 20,
                height: 2,
            },
            z: 0,
            kind: RegionKind::TranscriptMessage {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                message_idx: 1,
            },
            hover_cursor: HoverCursor::Text,
            keyboard_binding: None,
        });
        let content =
            DragSelectionContent::new().with_transcript_lines("message-1", ["hello world"]);
        let mut drag = DragSelectionController::new();

        assert!(drag.handle_event(
            &frame,
            &InteractiveEvent::DragStart {
                region: "message-1".to_string(),
                anchor: (1, 0),
            }
        ));
        assert!(drag.handle_event_with_content(
            &frame,
            &InteractiveEvent::DragEnd {
                region: "message-1".to_string(),
                cursor: (4, 0),
            },
            &content,
        ));

        assert_eq!(
            drag.finalized()
                .map(|selection| drag_selection_text(selection, &content)),
            Some("ello".to_string())
        );
        assert!(drag.active().is_none());
    }

    #[test]
    fn drag_end_clears_too_short_selection() {
        let frame = frame_with(InteractiveRegion {
            id: "message-1".to_string(),
            rect: RegionRect {
                x: 0,
                y: 0,
                width: 20,
                height: 1,
            },
            z: 0,
            kind: RegionKind::TranscriptMessage {
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
                message_idx: 1,
            },
            hover_cursor: HoverCursor::Text,
            keyboard_binding: None,
        });
        let content = DragSelectionContent::new().with_transcript_lines("message-1", ["hello"]);
        let mut drag = DragSelectionController::new();

        assert!(drag.handle_event(
            &frame,
            &InteractiveEvent::DragStart {
                region: "message-1".to_string(),
                anchor: (1, 0),
            }
        ));
        assert!(drag.handle_event_with_content(
            &frame,
            &InteractiveEvent::DragEnd {
                region: "message-1".to_string(),
                cursor: (2, 0),
            },
            &content,
        ));

        assert!(drag.finalized().is_none());
        assert!(drag.active().is_none());
    }
}
