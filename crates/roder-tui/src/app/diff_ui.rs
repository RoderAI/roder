use super::*;
use crate::diff::DiffViewerState;
use crate::diff::keys::{DiffKey, DiffKeyOutcome, apply_key};
use crate::diff::render::diff_viewer_widget;
use roder_api::interactive::{HoverCursor, InteractiveRegion, RegionKind, RegionRect};

impl TuiApp {
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
        self.apply_diff_key(diff_key).await;
    }

    async fn apply_diff_key(&mut self, diff_key: DiffKey) {
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

    pub(super) async fn handle_diff_region_click(&mut self, region_id: &str, hunk_idx: usize) {
        let Some(state) = self.diff_viewer.as_mut() else {
            return;
        };
        if state.hunk_count() == 0 {
            return;
        }
        state.hunk_index = hunk_idx.min(state.hunk_count() - 1);
        let key = if region_id.ends_with(":reject") {
            DiffKey::RejectHunk
        } else {
            DiffKey::AcceptHunk
        };
        self.apply_diff_key(key).await;
    }

    pub(super) async fn resolve_diff_approval(&mut self, approval_id: String, approved: bool) {
        let params = SessionResolveApprovalParams {
            approval_id: approval_id.clone(),
            approved,
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("session/resolve_approval")),
                method: "session/resolve_approval".to_string(),
                params: Some(serde_json::to_value(params).unwrap()),
            })
            .await;
        match decode_response::<SessionResolveApprovalResult>(res) {
            Ok(result) if result.resolved => {}
            Ok(_) => self.record_error(format!("approval not pending: {}", short_id(&approval_id))),
            Err(err) => self.record_error(format!("session/resolve_approval failed: {err}")),
        }
    }

    pub(super) fn render_diff_viewer(&self, f: &mut Frame<'_>, area: Rect) {
        let Some(state) = self.diff_viewer.as_ref() else {
            return;
        };
        let diff_area = diff_popup_area(area);
        f.render_widget(Clear, diff_area);
        f.render_widget(diff_viewer_widget(state, self.theme.diff()), diff_area);
    }

    pub(super) fn diff_hunk_regions(&self, area: Rect) -> Vec<InteractiveRegion> {
        let Some(state) = self.diff_viewer.as_ref() else {
            return Vec::new();
        };
        diff_hunk_regions_for_state(state, diff_popup_area(area))
    }
}

fn diff_popup_area(area: Rect) -> Rect {
    centered_rect(area, area.width.min(100), area.height.min(24))
}

fn diff_hunk_regions_for_state(state: &DiffViewerState, diff_area: Rect) -> Vec<InteractiveRegion> {
    let Some(file) = state.current_file() else {
        return Vec::new();
    };
    let inner = Rect {
        x: diff_area.x.saturating_add(1),
        y: diff_area.y.saturating_add(1),
        width: diff_area.width.saturating_sub(2),
        height: diff_area.height.saturating_sub(2),
    };
    let mut row = 2usize;
    let mut regions = Vec::new();
    for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
        if row >= usize::from(inner.height) {
            break;
        }
        let y = inner.y.saturating_add(row as u16);
        let reject_width = 10.min(inner.width);
        let accept_width = 10.min(inner.width.saturating_sub(reject_width));
        let reject_x = inner
            .x
            .saturating_add(inner.width.saturating_sub(reject_width));
        let accept_x = reject_x.saturating_sub(accept_width);
        regions.push(diff_hunk_region(
            state,
            hunk_idx,
            "accept",
            RegionRect {
                x: accept_x,
                y,
                width: accept_width,
                height: 1,
            },
        ));
        regions.push(diff_hunk_region(
            state,
            hunk_idx,
            "reject",
            RegionRect {
                x: reject_x,
                y,
                width: reject_width,
                height: 1,
            },
        ));
        row = row.saturating_add(1 + hunk.lines.len());
    }
    regions
}

fn diff_hunk_region(
    state: &DiffViewerState,
    hunk_idx: usize,
    action: &str,
    rect: RegionRect,
) -> InteractiveRegion {
    let file = state.current_file().expect("diff hunk region needs file");
    InteractiveRegion {
        id: format!("diff:{}:{hunk_idx}:{action}", state.pending.call_id),
        rect,
        z: 20,
        kind: RegionKind::DiffHunk {
            call_id: state.pending.call_id.clone(),
            file_path: file.path.clone(),
            hunk_idx,
        },
        hover_cursor: HoverCursor::Pointer,
        keyboard_binding: None,
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

    #[test]
    fn diff_hunk_regions_cover_accept_and_reject_controls() {
        let state = diff_region_state();

        let regions = diff_hunk_regions_for_state(&state, Rect::new(4, 2, 80, 12));

        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].id, "diff:call-a:0:accept");
        assert_eq!(regions[1].id, "diff:call-a:0:reject");
        assert!(matches!(
            regions[0].kind,
            RegionKind::DiffHunk { hunk_idx: 0, .. }
        ));
        assert_eq!(regions[0].rect.y, 5);
        assert_eq!(regions[1].rect.x, 73);
    }

    fn diff_region_state() -> DiffViewerState {
        use crate::diff::compute::compute_diff;
        use crate::diff::{FileDiff, PendingDiff};

        DiffViewerState::new(PendingDiff {
            call_id: "call-a".to_string(),
            tool: "edit".to_string(),
            files: vec![FileDiff {
                path: "src/lib.rs".into(),
                change_type: "modify".to_string(),
                before: Some("one\ntwo\nthree\n".to_string()),
                after: "one\nTWO\nthree\nfour\n".to_string(),
                supports_partial: true,
                hunks: compute_diff(Some("one\ntwo\nthree\n"), "one\nTWO\nthree\nfour\n"),
            }],
        })
    }
}
