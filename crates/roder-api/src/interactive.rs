use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::events::{ThreadId, TurnId};
use crate::extension::ExtensionId;

pub type RegionId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteractiveRegion {
    pub id: RegionId,
    pub rect: RegionRect,
    pub z: i16,
    pub kind: RegionKind,
    pub hover_cursor: HoverCursor,
    pub keyboard_binding: Option<KeyChord>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegionRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl RegionRect {
    pub fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RegionKind {
    TranscriptMessage {
        thread_id: ThreadId,
        turn_id: TurnId,
        message_idx: usize,
    },
    ToolCallBlock {
        call_id: String,
        expanded: bool,
    },
    FileReference {
        path: PathBuf,
        line: Option<u32>,
    },
    Url(String),
    AttachmentThumbnail {
        attachment_id: String,
    },
    StatusSegment {
        segment_id: String,
    },
    PaletteItem {
        source_id: String,
        item_id: String,
    },
    DiffHunk {
        call_id: String,
        file_path: PathBuf,
        hunk_idx: usize,
    },
    PolicyApprovalButton {
        decision_id: String,
        vote: ApprovalVote,
    },
    Composer,
    Custom {
        extension_id: ExtensionId,
        payload: serde_json::Value,
    },
}

impl RegionKind {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::TranscriptMessage { .. } => "TranscriptMessage",
            Self::ToolCallBlock { .. } => "ToolCallBlock",
            Self::FileReference { .. } => "FileReference",
            Self::Url(_) => "Url",
            Self::AttachmentThumbnail { .. } => "AttachmentThumbnail",
            Self::StatusSegment { .. } => "StatusSegment",
            Self::PaletteItem { .. } => "PaletteItem",
            Self::DiffHunk { .. } => "DiffHunk",
            Self::PolicyApprovalButton { .. } => "PolicyApprovalButton",
            Self::Composer => "Composer",
            Self::Custom { .. } => "Custom",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApprovalVote {
    Approve,
    Deny,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HoverCursor {
    Default,
    Pointer,
    Text,
    Grab,
    Crosshair,
    NotAllowed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyChord {
    pub key: String,
    #[serde(default)]
    pub modifiers: InteractiveModifiers,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteractiveModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_key: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InteractiveMouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum InteractiveEvent {
    HoverEnter {
        region: RegionId,
    },
    HoverLeave {
        region: RegionId,
    },
    Click {
        region: RegionId,
        modifiers: InteractiveModifiers,
        button: InteractiveMouseButton,
    },
    DoubleClick {
        region: RegionId,
        modifiers: InteractiveModifiers,
    },
    RightClick {
        region: RegionId,
        modifiers: InteractiveModifiers,
    },
    DragStart {
        region: RegionId,
        anchor: (u16, u16),
    },
    DragUpdate {
        region: RegionId,
        cursor: (u16, u16),
    },
    DragEnd {
        region: RegionId,
        cursor: (u16, u16),
    },
    Scroll {
        region: Option<RegionId>,
        delta_lines: i16,
        modifiers: InteractiveModifiers,
    },
}

#[async_trait::async_trait]
pub trait InteractiveRegionHandler: Send + Sync + 'static {
    fn id(&self) -> String;

    fn kinds(&self) -> &[&'static str];

    async fn handle(
        &self,
        event: InteractiveEvent,
        region: &InteractiveRegion,
    ) -> anyhow::Result<HandlerOutcome>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HandlerOutcome {
    Consumed,
    Passthrough,
    InvalidateRender,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_rect_contains_inside_edges_only() {
        let rect = RegionRect {
            x: 2,
            y: 3,
            width: 4,
            height: 2,
        };

        assert!(rect.contains(2, 3));
        assert!(rect.contains(5, 4));
        assert!(!rect.contains(6, 4));
        assert!(!rect.contains(5, 5));
    }

    #[test]
    fn interactive_region_round_trips_json() {
        let region = InteractiveRegion {
            id: "region-1".to_string(),
            rect: RegionRect {
                x: 0,
                y: 1,
                width: 10,
                height: 2,
            },
            z: 3,
            kind: RegionKind::ToolCallBlock {
                call_id: "call-1".to_string(),
                expanded: false,
            },
            hover_cursor: HoverCursor::Pointer,
            keyboard_binding: Some(KeyChord {
                key: "enter".to_string(),
                modifiers: InteractiveModifiers::default(),
            }),
        };

        let encoded = serde_json::to_value(&region).unwrap();
        let decoded: InteractiveRegion = serde_json::from_value(encoded).unwrap();

        assert_eq!(decoded, region);
    }
}
