use crate::diff::compute::HunkStatus;
use crate::diff::{DiffResolution, DiffViewMode, DiffViewerState};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DiffKey {
    NextHunk,
    PreviousHunk,
    AcceptHunk,
    RejectHunk,
    AcceptAll,
    RejectAll,
    ToggleView,
    Close,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DiffKeyOutcome {
    Consumed,
    AcceptedWhole,
    RejectedWhole,
    Closed,
}

pub fn apply_key(state: &mut DiffViewerState, key: DiffKey) -> DiffKeyOutcome {
    match key {
        DiffKey::NextHunk => {
            let count = state.hunk_count();
            if count > 0 {
                state.hunk_index = (state.hunk_index + 1).min(count - 1);
            }
            DiffKeyOutcome::Consumed
        }
        DiffKey::PreviousHunk => {
            state.hunk_index = state.hunk_index.saturating_sub(1);
            DiffKeyOutcome::Consumed
        }
        DiffKey::AcceptHunk => {
            if state.supports_partial() {
                if let Some(hunk) = state.current_hunk_mut() {
                    hunk.status = HunkStatus::Accepted;
                }
                DiffKeyOutcome::Consumed
            } else {
                accept_whole(state)
            }
        }
        DiffKey::RejectHunk => {
            if state.supports_partial() {
                if let Some(hunk) = state.current_hunk_mut() {
                    hunk.status = HunkStatus::Rejected;
                }
                DiffKeyOutcome::Consumed
            } else {
                reject_whole(state)
            }
        }
        DiffKey::AcceptAll => accept_whole(state),
        DiffKey::RejectAll => reject_whole(state),
        DiffKey::ToggleView => {
            state.mode = match state.mode {
                DiffViewMode::Unified => DiffViewMode::SideBySide,
                DiffViewMode::SideBySide => DiffViewMode::Unified,
            };
            state.clamp_cursor();
            DiffKeyOutcome::Consumed
        }
        DiffKey::Close => DiffKeyOutcome::Closed,
    }
}

fn accept_whole(state: &mut DiffViewerState) -> DiffKeyOutcome {
    state.set_all_hunks(HunkStatus::Accepted);
    state.resolution = Some(DiffResolution::Accepted);
    DiffKeyOutcome::AcceptedWhole
}

fn reject_whole(state: &mut DiffViewerState) -> DiffKeyOutcome {
    state.set_all_hunks(HunkStatus::Rejected);
    state.resolution = Some(DiffResolution::Rejected);
    DiffKeyOutcome::RejectedWhole
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::compute::compute_diff;
    use crate::diff::{DiffViewerState, FileDiff, PendingDiff};

    #[test]
    fn diff_view_hunk_navigation_clamps_at_edges() {
        let mut state = state(true);
        apply_key(&mut state, DiffKey::NextHunk);
        apply_key(&mut state, DiffKey::NextHunk);
        assert_eq!(state.hunk_index, state.hunk_count() - 1);
        apply_key(&mut state, DiffKey::PreviousHunk);
        assert_eq!(state.hunk_index, 0);
        apply_key(&mut state, DiffKey::PreviousHunk);
        assert_eq!(state.hunk_index, 0);
    }

    #[test]
    fn diff_view_partial_accept_reject_updates_current_hunk_status() {
        let mut state = state(true);
        assert_eq!(
            apply_key(&mut state, DiffKey::AcceptHunk),
            DiffKeyOutcome::Consumed
        );
        assert_eq!(state.current_hunk().unwrap().status, HunkStatus::Accepted);
        assert_eq!(
            apply_key(&mut state, DiffKey::RejectHunk),
            DiffKeyOutcome::Consumed
        );
        assert_eq!(state.current_hunk().unwrap().status, HunkStatus::Rejected);
    }

    #[test]
    fn diff_view_whole_file_fallback_resolves_when_partial_is_unsupported() {
        let mut state = state(false);
        assert_eq!(
            apply_key(&mut state, DiffKey::AcceptHunk),
            DiffKeyOutcome::AcceptedWhole
        );
        assert_eq!(state.resolution, Some(DiffResolution::Accepted));
        assert!(
            state
                .pending
                .files
                .iter()
                .flat_map(|file| &file.hunks)
                .all(|hunk| hunk.status == HunkStatus::Accepted)
        );
    }

    #[test]
    fn diff_view_toggle_preserves_cursor_and_statuses() {
        let mut state = state(true);
        apply_key(&mut state, DiffKey::AcceptHunk);
        apply_key(&mut state, DiffKey::ToggleView);
        assert_eq!(state.mode, DiffViewMode::SideBySide);
        assert_eq!(state.hunk_index, 0);
        assert_eq!(state.current_hunk().unwrap().status, HunkStatus::Accepted);
    }

    fn state(supports_partial: bool) -> DiffViewerState {
        DiffViewerState::new(PendingDiff {
            call_id: "call-a".to_string(),
            tool: "edit".to_string(),
            files: vec![FileDiff {
                path: "src/lib.rs".into(),
                change_type: "modify".to_string(),
                before: Some("one\ntwo\nthree\n".to_string()),
                after: "one\nTWO\nthree\nfour\n".to_string(),
                supports_partial,
                hunks: compute_diff(Some("one\ntwo\nthree\n"), "one\nTWO\nthree\nfour\n"),
            }],
        })
    }
}
