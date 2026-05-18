use std::path::PathBuf;

use ratatui::layout::Rect;
use roder_api::interactive::{
    ApprovalVote, HoverCursor, InteractiveRegion, RegionId, RegionKind, RegionRect,
};

pub fn status_segment_region(
    segment_id: impl Into<String>,
    rect: Rect,
    z: i16,
) -> InteractiveRegion {
    let segment_id = segment_id.into();
    InteractiveRegion {
        id: region_id("status", segment_id.clone()),
        rect: to_region_rect(rect),
        z,
        kind: RegionKind::StatusSegment { segment_id },
        hover_cursor: HoverCursor::Pointer,
        keyboard_binding: None,
    }
}

pub fn palette_item_region(
    source_id: impl Into<String>,
    item_id: impl Into<String>,
    rect: Rect,
    z: i16,
) -> InteractiveRegion {
    let source_id = source_id.into();
    let item_id = item_id.into();
    InteractiveRegion {
        id: region_id("palette", format!("{source_id}:{item_id}")),
        rect: to_region_rect(rect),
        z,
        kind: RegionKind::PaletteItem { source_id, item_id },
        hover_cursor: HoverCursor::Pointer,
        keyboard_binding: None,
    }
}

pub fn diff_hunk_region(
    call_id: impl Into<String>,
    file_path: impl Into<PathBuf>,
    hunk_idx: usize,
    rect: Rect,
    z: i16,
) -> InteractiveRegion {
    let call_id = call_id.into();
    let file_path = file_path.into();
    InteractiveRegion {
        id: region_id("diff", format!("{call_id}:{hunk_idx}")),
        rect: to_region_rect(rect),
        z,
        kind: RegionKind::DiffHunk {
            call_id,
            file_path,
            hunk_idx,
        },
        hover_cursor: HoverCursor::Pointer,
        keyboard_binding: None,
    }
}

pub fn policy_approval_region(
    decision_id: impl Into<String>,
    vote: ApprovalVote,
    rect: Rect,
    z: i16,
) -> InteractiveRegion {
    let decision_id = decision_id.into();
    InteractiveRegion {
        id: region_id("approval", format!("{decision_id}:{vote:?}")),
        rect: to_region_rect(rect),
        z,
        kind: RegionKind::PolicyApprovalButton { decision_id, vote },
        hover_cursor: HoverCursor::Pointer,
        keyboard_binding: None,
    }
}

fn region_id(prefix: &str, key: String) -> RegionId {
    format!("{prefix}:{key}")
}

fn to_region_rect(rect: Rect) -> RegionRect {
    RegionRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

#[cfg(test)]
mod tests {
    use roder_api::interactive::ApprovalVote;

    use super::*;

    #[test]
    fn mouse_integration_builtins_register_clickable_region_kinds() {
        let status = status_segment_region("model", Rect::new(1, 2, 10, 1), 10);
        assert!(matches!(
            status.kind,
            RegionKind::StatusSegment { ref segment_id } if segment_id == "model"
        ));

        let palette = palette_item_region("models", "gpt-5.5", Rect::new(2, 3, 20, 1), 20);
        assert!(matches!(
            palette.kind,
            RegionKind::PaletteItem {
                ref source_id,
                ref item_id
            } if source_id == "models" && item_id == "gpt-5.5"
        ));

        let diff = diff_hunk_region("call-1", "src/main.rs", 2, Rect::new(3, 4, 30, 1), 30);
        assert!(matches!(
            diff.kind,
            RegionKind::DiffHunk {
                ref call_id,
                ref file_path,
                hunk_idx: 2
            } if call_id == "call-1" && file_path == &PathBuf::from("src/main.rs")
        ));

        let approval = policy_approval_region(
            "approval-1",
            ApprovalVote::Approve,
            Rect::new(4, 5, 8, 1),
            40,
        );
        assert!(matches!(
            approval.kind,
            RegionKind::PolicyApprovalButton {
                ref decision_id,
                vote: ApprovalVote::Approve
            } if decision_id == "approval-1"
        ));
    }
}
