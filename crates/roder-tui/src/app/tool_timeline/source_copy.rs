use crate::transcript::source_copy::SourceCopyMode;

use super::{TimelineItemKind, TimelineState};

impl TimelineState {
    pub(in crate::app) fn sync_source_copy(&self, mode: &mut SourceCopyMode) {
        mode.sync(self.items.iter().filter_map(|item| match &item.kind {
            TimelineItemKind::Assistant(message) => {
                Some((message.source_id, message.text.as_str()))
            }
            // Tool/image controls, reserved raster rows and status text are not
            // assistant source and must never enter copy targets.
            _ => None,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::scroll_accel::ScrollSettings;
    use crate::app::tool_timeline::TimelineSettings;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn sources_ignore_tools_and_keep_identity_across_streaming_and_prepend() {
        let mut timeline =
            TimelineState::new(ScrollSettings::default(), TimelineSettings::default());
        timeline.push_assistant_delta_immediate("```rs\r\nx  ", None);
        timeline.push_system("image preview controls");
        let mut mode = SourceCopyMode::default();
        timeline.sync_source_copy(&mut mode);
        mode.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let selected = mode.selected_id();
        assert_eq!(mode.selected_text(), Some("x  "));
        // Removing unrelated items is not a source mutation.
        timeline.items.pop();
        timeline.push_assistant_delta_immediate("\r\n```\n", None);
        timeline.prepend_assistant("older source", None);
        timeline.sync_source_copy(&mut mode);
        assert_eq!(mode.selected_id(), selected);
        assert_eq!(mode.selected_text(), Some("x  \r\n"));
        assert_eq!(mode.selected_position(), Some((3, 3)));
    }
}
