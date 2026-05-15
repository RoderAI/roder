use roder_api::interactive::{InteractiveEvent, InteractiveRegion, RegionKind};

use crate::selection::{OffsetRange, Point, Range, selected_offset_text, selected_text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionTarget {
    Transcript {
        start_message_idx: usize,
        origin: (u16, u16),
    },
    Composer {
        origin: (u16, u16),
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectedText {
    Transcript(String),
    Composer(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragSelectionOutcome {
    Started(SelectionTarget),
    Updated,
    Finalized(SelectedText),
    ClearedTooShort,
}

#[derive(Debug, Clone)]
pub struct DragSelectionState {
    target: Option<SelectionTarget>,
    transcript: Range,
    composer: OffsetRange,
    min_chars: usize,
}

impl Default for DragSelectionState {
    fn default() -> Self {
        Self {
            target: None,
            transcript: Range::default(),
            composer: OffsetRange::default(),
            min_chars: 3,
        }
    }
}

impl DragSelectionState {
    pub fn with_min_chars(mut self, min_chars: usize) -> Self {
        self.min_chars = min_chars;
        self
    }

    pub fn apply_event(
        &mut self,
        event: &InteractiveEvent,
        region: Option<&InteractiveRegion>,
        transcript_lines: &[String],
        composer_text: &str,
    ) -> Option<DragSelectionOutcome> {
        match event {
            InteractiveEvent::DragStart { anchor, .. } => {
                let region = region?;
                self.start(region, *anchor)
            }
            InteractiveEvent::DragUpdate { cursor, .. } => self.update(*cursor),
            InteractiveEvent::DragEnd { cursor, .. } => {
                self.update(*cursor);
                self.finalize(transcript_lines, composer_text)
            }
            _ => None,
        }
    }

    pub fn transcript_range(&self) -> Option<Range> {
        self.transcript.active.then_some(self.transcript)
    }

    pub fn composer_range(&self) -> Option<OffsetRange> {
        self.composer.active.then_some(self.composer)
    }

    pub fn clear(&mut self) {
        self.target = None;
        self.transcript.clear();
        self.composer.clear();
    }

    fn start(
        &mut self,
        region: &InteractiveRegion,
        anchor: (u16, u16),
    ) -> Option<DragSelectionOutcome> {
        match &region.kind {
            RegionKind::TranscriptMessage { message_idx, .. } => {
                let point = transcript_point(*message_idx, region.rect.x, anchor);
                self.target = Some(SelectionTarget::Transcript {
                    start_message_idx: *message_idx,
                    origin: (region.rect.x, region.rect.y),
                });
                self.transcript = Range::new(point);
                self.composer.clear();
                Some(DragSelectionOutcome::Started(self.target.clone()?))
            }
            RegionKind::Composer => {
                let offset = anchor.0.saturating_sub(region.rect.x) as usize;
                self.target = Some(SelectionTarget::Composer {
                    origin: (region.rect.x, region.rect.y),
                });
                self.composer = OffsetRange::new(offset);
                self.transcript.clear();
                Some(DragSelectionOutcome::Started(self.target.clone()?))
            }
            _ => None,
        }
    }

    fn update(&mut self, cursor: (u16, u16)) -> Option<DragSelectionOutcome> {
        match self.target.clone()? {
            SelectionTarget::Transcript {
                start_message_idx,
                origin,
            } => {
                let line_delta = cursor.1.saturating_sub(origin.1) as usize;
                let line = start_message_idx.saturating_add(line_delta);
                self.transcript.update(Point {
                    line,
                    col: cursor.0.saturating_sub(origin.0) as usize,
                });
                Some(DragSelectionOutcome::Updated)
            }
            SelectionTarget::Composer { origin } => {
                self.composer
                    .update(cursor.0.saturating_sub(origin.0) as usize);
                Some(DragSelectionOutcome::Updated)
            }
        }
    }

    fn finalize(
        &mut self,
        transcript_lines: &[String],
        composer_text: &str,
    ) -> Option<DragSelectionOutcome> {
        let target = self.target.clone()?;
        let selected = match target {
            SelectionTarget::Transcript { .. } => {
                let text = selected_text(transcript_lines, self.transcript);
                if text.chars().count() < self.min_chars {
                    self.clear();
                    return Some(DragSelectionOutcome::ClearedTooShort);
                }
                SelectedText::Transcript(text)
            }
            SelectionTarget::Composer { .. } => {
                let text = selected_offset_text(composer_text, self.composer);
                if text.chars().count() < self.min_chars {
                    self.clear();
                    return Some(DragSelectionOutcome::ClearedTooShort);
                }
                SelectedText::Composer(text)
            }
        };
        self.clear();
        Some(DragSelectionOutcome::Finalized(selected))
    }
}

fn transcript_point(message_idx: usize, origin_x: u16, at: (u16, u16)) -> Point {
    Point {
        line: message_idx,
        col: at.0.saturating_sub(origin_x) as usize,
    }
}
