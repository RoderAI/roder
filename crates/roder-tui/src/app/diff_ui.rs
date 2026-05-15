use super::*;
use crate::diff::DiffViewerState;
use crate::diff::keys::{DiffKey, DiffKeyOutcome, apply_key};
use crate::diff::render::diff_viewer_widget;

impl TuiApp {
    pub(super) fn open_diff_preview(&mut self, preview: roder_api::events::FileChangePreviewReady) {
        self.show_palette = false;
        self.show_provider_popup = false;
        let path = preview.path.clone();
        self.diff_viewer = Some(DiffViewerState::from_preview(preview));
        self.push_event(format!("diff preview ready: {path}"));
    }

    pub(super) fn handle_diff_key(&mut self, key: crossterm::event::KeyEvent) {
        let Some(diff_key) = diff_key_from_event(key) else {
            return;
        };
        let Some(state) = self.diff_viewer.as_mut() else {
            return;
        };
        let outcome = apply_key(state, diff_key);
        match outcome {
            DiffKeyOutcome::Consumed => {}
            DiffKeyOutcome::AcceptedWhole => {
                let call_id = state.pending.call_id.clone();
                self.diff_viewer = None;
                self.push_event(format!("diff accepted: {call_id}"));
            }
            DiffKeyOutcome::RejectedWhole => {
                let call_id = state.pending.call_id.clone();
                self.diff_viewer = None;
                self.push_event(format!("diff rejected: {call_id}"));
            }
            DiffKeyOutcome::Closed => {
                self.diff_viewer = None;
            }
        }
    }

    pub(super) fn render_diff_viewer(&self, f: &mut Frame<'_>, area: Rect) {
        let Some(state) = self.diff_viewer.as_ref() else {
            return;
        };
        let diff_area = centered_rect(area, area.width.min(100), area.height.min(24));
        f.render_widget(Clear, diff_area);
        f.render_widget(diff_viewer_widget(state, self.theme.diff()), diff_area);
    }
}

fn diff_key_from_event(key: crossterm::event::KeyEvent) -> Option<DiffKey> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(DiffKey::NextHunk),
        KeyCode::Char('k') | KeyCode::Up => Some(DiffKey::PreviousHunk),
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(DiffKey::AcceptHunk),
        KeyCode::Char('n') | KeyCode::Char('N') => Some(DiffKey::RejectHunk),
        KeyCode::Char('a') | KeyCode::Char('A') => Some(DiffKey::AcceptAll),
        KeyCode::Char('r') | KeyCode::Char('R') => Some(DiffKey::RejectAll),
        KeyCode::Char('s') | KeyCode::Char('S') => Some(DiffKey::ToggleView),
        KeyCode::Esc => Some(DiffKey::Close),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_view_key_mapping_covers_documented_keys() {
        assert_eq!(
            diff_key_from_event(crossterm::event::KeyEvent::new(
                KeyCode::Char('j'),
                KeyModifiers::NONE,
            )),
            Some(DiffKey::NextHunk)
        );
        assert_eq!(
            diff_key_from_event(crossterm::event::KeyEvent::new(
                KeyCode::Char('s'),
                KeyModifiers::NONE,
            )),
            Some(DiffKey::ToggleView)
        );
        assert_eq!(
            diff_key_from_event(crossterm::event::KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )),
            Some(DiffKey::Close)
        );
    }
}
