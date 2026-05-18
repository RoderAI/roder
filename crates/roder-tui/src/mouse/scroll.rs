use roder_api::interactive::{InteractiveEvent, RegionId, RegionKind};

use super::regions::RegionFrame;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScrollTarget {
    #[default]
    Transcript,
    Palette,
    Diff,
    Monitor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollCommand {
    pub target: ScrollTarget,
    pub delta_lines: i16,
}

#[derive(Debug, Clone, Default)]
pub struct ScrollController {
    focused_target: Option<ScrollTarget>,
}

impl ScrollController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_focus(&mut self, target: Option<ScrollTarget>) {
        self.focused_target = target;
    }

    pub fn command_for_event(
        &self,
        frame: &RegionFrame,
        event: &InteractiveEvent,
    ) -> Option<ScrollCommand> {
        let InteractiveEvent::Scroll {
            region,
            delta_lines,
            ..
        } = event
        else {
            return None;
        };

        Some(ScrollCommand {
            target: self.resolve_target(frame, region.as_ref()),
            delta_lines: *delta_lines,
        })
    }

    fn resolve_target(&self, frame: &RegionFrame, region_id: Option<&RegionId>) -> ScrollTarget {
        if let Some(target) = region_id
            .and_then(|id| frame.get(id))
            .and_then(region_scroll_target)
        {
            return target;
        }
        self.focused_target.unwrap_or(ScrollTarget::Transcript)
    }
}

fn region_scroll_target(
    region: &roder_api::interactive::InteractiveRegion,
) -> Option<ScrollTarget> {
    match &region.kind {
        RegionKind::TranscriptMessage { .. }
        | RegionKind::ToolCallBlock { .. }
        | RegionKind::Url(_)
        | RegionKind::FileReference { .. } => Some(ScrollTarget::Transcript),
        RegionKind::PaletteItem { .. } => Some(ScrollTarget::Palette),
        RegionKind::DiffHunk { .. } => Some(ScrollTarget::Diff),
        RegionKind::Custom { payload, .. }
            if payload.get("surface").and_then(|value| value.as_str()) == Some("monitor") =>
        {
            Some(ScrollTarget::Monitor)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use roder_api::interactive::{
        HoverCursor, InteractiveModifiers, InteractiveRegion, RegionRect,
    };

    use super::*;

    fn region(id: &str, kind: RegionKind) -> InteractiveRegion {
        InteractiveRegion {
            id: id.to_string(),
            rect: RegionRect {
                x: 0,
                y: 0,
                width: 10,
                height: 1,
            },
            z: 0,
            kind,
            hover_cursor: HoverCursor::Default,
            keyboard_binding: None,
        }
    }

    #[test]
    fn scroll_defaults_to_transcript_without_focus_or_region() {
        let controller = ScrollController::new();
        let frame = RegionFrame::new();

        assert_eq!(
            controller.command_for_event(
                &frame,
                &InteractiveEvent::Scroll {
                    region: None,
                    delta_lines: 3,
                    modifiers: InteractiveModifiers::default(),
                }
            ),
            Some(ScrollCommand {
                target: ScrollTarget::Transcript,
                delta_lines: 3,
            })
        );
    }

    #[test]
    fn scroll_uses_focused_target_when_region_is_unknown() {
        let mut controller = ScrollController::new();
        controller.set_focus(Some(ScrollTarget::Palette));
        let frame = RegionFrame::new();

        assert_eq!(
            controller.command_for_event(
                &frame,
                &InteractiveEvent::Scroll {
                    region: Some("missing".to_string()),
                    delta_lines: -3,
                    modifiers: InteractiveModifiers::default(),
                }
            ),
            Some(ScrollCommand {
                target: ScrollTarget::Palette,
                delta_lines: -3,
            })
        );
    }

    #[test]
    fn scroll_region_overrides_focus_for_diff_and_monitor() {
        let mut controller = ScrollController::new();
        controller.set_focus(Some(ScrollTarget::Palette));
        let mut frame = RegionFrame::new();
        frame.push(region(
            "hunk",
            RegionKind::DiffHunk {
                call_id: "call-1".to_string(),
                file_path: "src/lib.rs".into(),
                hunk_idx: 0,
            },
        ));
        frame.push(region(
            "monitor",
            RegionKind::Custom {
                extension_id: "roder-tui".to_string(),
                payload: serde_json::json!({ "surface": "monitor" }),
            },
        ));

        assert_eq!(
            controller
                .command_for_event(
                    &frame,
                    &InteractiveEvent::Scroll {
                        region: Some("hunk".to_string()),
                        delta_lines: 3,
                        modifiers: InteractiveModifiers::default(),
                    }
                )
                .map(|command| command.target),
            Some(ScrollTarget::Diff)
        );
        assert_eq!(
            controller
                .command_for_event(
                    &frame,
                    &InteractiveEvent::Scroll {
                        region: Some("monitor".to_string()),
                        delta_lines: 3,
                        modifiers: InteractiveModifiers::default(),
                    }
                )
                .map(|command| command.target),
            Some(ScrollTarget::Monitor)
        );
    }
}
