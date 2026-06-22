use super::*;
use roder_tui_diff::DiffViewerState;
use roder_tui_diff::keys::{DiffKey, DiffKeyOutcome, apply_key};
use roder_tui_diff::render::diff_viewer_widget;

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) fn open_diff_preview(&mut self, preview: roder_api::events::FileChangePreviewReady) {
        self.show_palette = false;
        self.show_provider_popup = false;
        let path = preview.path.clone();
        self.diff_viewer = Some(DiffViewerState::from_preview(preview));
        self.push_event(format!("diff preview ready: {path}"));
    }

    pub(super) async fn handle_diff_key(&mut self, key: crossterm::event::KeyEvent) {
        let Some(diff_key) = diff_key_from_event(key) else {
            return;
        };
        let outcome = {
            let Some(state) = self.diff_viewer.as_mut() else {
                return;
            };
            apply_key(state, diff_key)
        };
        match outcome {
            DiffKeyOutcome::Consumed => {}
            DiffKeyOutcome::AcceptedWhole => {
                let Some(call_id) = self
                    .diff_viewer
                    .as_ref()
                    .map(|state| state.pending.call_id.clone())
                else {
                    return;
                };
                self.diff_viewer = None;
                self.resolve_diff_approval(call_id.clone(), true).await;
                self.push_event(format!("diff accepted: {call_id}"));
            }
            DiffKeyOutcome::RejectedWhole => {
                let Some(call_id) = self
                    .diff_viewer
                    .as_ref()
                    .map(|state| state.pending.call_id.clone())
                else {
                    return;
                };
                self.diff_viewer = None;
                self.resolve_diff_approval(call_id.clone(), false).await;
                self.push_event(format!("diff rejected: {call_id}"));
            }
            DiffKeyOutcome::Closed => {
                let Some(call_id) = self
                    .diff_viewer
                    .as_ref()
                    .map(|state| state.pending.call_id.clone())
                else {
                    return;
                };
                self.diff_viewer = None;
                self.resolve_diff_approval(call_id.clone(), false).await;
                self.push_event(format!("diff closed: {call_id}"));
            }
        }
    }

    pub(super) async fn resolve_diff_approval(&mut self, approval_id: String, approved: bool) {
        let params = ThreadResolveApprovalParams {
            approval_id: approval_id.clone(),
            approved,
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("thread/resolve_approval")),
                method: "thread/resolve_approval".to_string(),
                params: Some(serde_json::to_value(params).unwrap()),
            })
            .await;
        match decode_response::<ThreadResolveApprovalResult>(res) {
            Ok(result) if result.resolved => {}
            Ok(_) => self.record_error(format!("approval not pending: {}", short_id(&approval_id))),
            Err(err) => self.record_error(format!("thread/resolve_approval failed: {err}")),
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
