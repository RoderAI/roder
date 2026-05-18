use ratatui::layout::Rect;
use roder_api::interactive::{HoverCursor, InteractiveRegion, RegionId, RegionKind, RegionRect};
use serde::{Deserialize, Serialize};

const MENU_WIDTH: u16 = 22;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptContextMenuAction {
    Copy,
    Expand,
    JumpToToolCall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptContextMenu {
    anchor_region: RegionId,
    rect: RegionRect,
    entries: Vec<TranscriptContextMenuEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TranscriptContextMenuEntry {
    action: TranscriptContextMenuAction,
    label: &'static str,
}

impl TranscriptContextMenu {
    pub fn at_message(
        anchor_region: impl Into<RegionId>,
        anchor: (u16, u16),
        viewport: Rect,
        can_expand: bool,
        can_jump_to_tool_call: bool,
    ) -> Self {
        let mut entries = vec![TranscriptContextMenuEntry {
            action: TranscriptContextMenuAction::Copy,
            label: "Copy message",
        }];
        if can_expand {
            entries.push(TranscriptContextMenuEntry {
                action: TranscriptContextMenuAction::Expand,
                label: "Expand/collapse",
            });
        }
        if can_jump_to_tool_call {
            entries.push(TranscriptContextMenuEntry {
                action: TranscriptContextMenuAction::JumpToToolCall,
                label: "Jump to tool call",
            });
        }

        let height = entries.len() as u16;
        let max_x = viewport
            .x
            .saturating_add(viewport.width)
            .saturating_sub(MENU_WIDTH);
        let max_y = viewport
            .y
            .saturating_add(viewport.height)
            .saturating_sub(height);
        let x = anchor.0.clamp(viewport.x, max_x.max(viewport.x));
        let y = anchor.1.clamp(viewport.y, max_y.max(viewport.y));

        Self {
            anchor_region: anchor_region.into(),
            rect: RegionRect {
                x,
                y,
                width: MENU_WIDTH,
                height,
            },
            entries,
        }
    }

    pub fn regions(&self) -> Vec<InteractiveRegion> {
        self.entries
            .iter()
            .enumerate()
            .map(|(index, entry)| InteractiveRegion {
                id: format!("context-menu-{}-{index}", self.anchor_region),
                rect: RegionRect {
                    x: self.rect.x,
                    y: self.rect.y.saturating_add(index as u16),
                    width: self.rect.width,
                    height: 1,
                },
                z: 100,
                kind: RegionKind::Custom {
                    extension_id: "roder-tui/transcript-context-menu".to_string(),
                    payload: serde_json::json!({
                        "anchor_region": self.anchor_region,
                        "action": entry.action,
                        "label": entry.label,
                    }),
                },
                hover_cursor: HoverCursor::Pointer,
                keyboard_binding: None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_menu_positions_inside_terminal_edges() {
        let menu = TranscriptContextMenu::at_message(
            "message-1",
            (79, 9),
            Rect::new(0, 0, 80, 10),
            true,
            true,
        );
        let regions = menu.regions();

        assert_eq!(regions.len(), 3);
        assert!(
            regions
                .iter()
                .all(|region| region.rect.x + region.rect.width <= 80)
        );
        assert!(regions.iter().all(|region| region.rect.y < 10));
    }
}
