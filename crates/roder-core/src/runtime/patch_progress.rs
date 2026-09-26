use roder_api::patch_progress::{PatchProgress, ProposedPatchChange};
use roder_edit_core::{Hunk, StreamingPatchParser, parse_patch};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct PatchProgressTracker {
    calls: HashMap<String, PendingPatch>,
}

#[derive(Default)]
struct PendingPatch {
    patch: String,
    parser: StreamingPatchParser,
    parsed_bytes: usize,
    last_emitted: Option<Instant>,
    invalid: bool,
}

impl PatchProgressTracker {
    pub(super) fn start(&mut self, id: &str, name: &str) {
        if name == "apply_patch" {
            self.calls.entry(id.to_string()).or_default();
        }
    }

    pub(super) fn delta(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        id: &str,
        delta: &str,
    ) -> Option<PatchProgress> {
        let pending = self.calls.get_mut(id)?;
        if pending.invalid {
            return None;
        }
        pending.patch.push_str(delta);
        if pending.patch.len() > 16 * 1024 * 1024 {
            pending.invalid = true;
            return None;
        }
        // JSON function-channel arguments are parsed at completion. The custom
        // channel sends raw patch text and can expose incremental hunks.
        if pending.patch.lines().next().map(str::trim) != Some("*** Begin Patch")
            || !pending.patch.contains('\n')
        {
            return None;
        }
        let hunks = match pending
            .parser
            .push_delta(&pending.patch[pending.parsed_bytes..])
        {
            Ok(hunks) => hunks,
            Err(_) => {
                pending.invalid = true;
                return None;
            }
        };
        pending.parsed_bytes = pending.patch.len();
        if pending.parser.environment_id().is_some() || hunks.is_empty() {
            return None;
        }
        if pending
            .last_emitted
            .is_some_and(|last| last.elapsed() < Duration::from_millis(250))
        {
            return None;
        }
        pending.last_emitted = Some(Instant::now());
        Some(progress(
            thread_id,
            turn_id,
            id,
            pending.patch.clone(),
            &hunks,
            false,
        ))
    }

    pub(super) fn complete(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        id: &str,
        name: &str,
        arguments: &str,
    ) -> Option<PatchProgress> {
        self.calls.remove(id);
        if name != "apply_patch" {
            return None;
        }
        let arguments: serde_json::Value = serde_json::from_str(arguments).ok()?;
        let patch = arguments.get("patch")?.as_str()?;
        let parsed = parse_patch(patch).ok()?;
        if parsed.environment_id.is_some() {
            return None;
        }
        Some(progress(
            thread_id,
            turn_id,
            id,
            patch.to_string(),
            &parsed.hunks,
            true,
        ))
    }
}

fn progress(
    thread: &str,
    turn: &str,
    id: &str,
    patch: String,
    hunks: &[Hunk],
    complete: bool,
) -> PatchProgress {
    let changes = hunks
        .iter()
        .map(|hunk| match hunk {
            Hunk::AddFile { path, contents } => ProposedPatchChange {
                path: path.display().to_string(),
                change_type: "add".into(),
                move_to: None,
                old_lines: vec![],
                new_lines: contents.lines().map(String::from).collect(),
            },
            Hunk::DeleteFile { path } => ProposedPatchChange {
                path: path.display().to_string(),
                change_type: "delete".into(),
                move_to: None,
                old_lines: vec![],
                new_lines: vec![],
            },
            Hunk::UpdateFile {
                path,
                move_path,
                chunks,
            } => ProposedPatchChange {
                path: path.display().to_string(),
                change_type: "update".into(),
                move_to: move_path.as_ref().map(|path| path.display().to_string()),
                old_lines: chunks
                    .iter()
                    .flat_map(|chunk| chunk.old_lines.clone())
                    .collect(),
                new_lines: chunks
                    .iter()
                    .flat_map(|chunk| chunk.new_lines.clone())
                    .collect(),
            },
        })
        .collect();
    PatchProgress {
        thread_id: thread.into(),
        turn_id: turn.into(),
        tool_id: id.into(),
        patch,
        changes,
        complete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streamed_preview_uses_requested_lines_and_reads_no_files() {
        let mut tracker = PatchProgressTracker::default();
        tracker.start("patch1", "apply_patch");
        let partial = tracker
            .delta(
                "thread",
                "turn",
                "patch1",
                "*** Begin Patch\n*** Update File: missing.txt\n@@\n-old\n+new\n",
            )
            .unwrap();
        assert_eq!(partial.changes[0].old_lines, vec!["old"]);
        assert_eq!(partial.changes[0].new_lines, vec!["new"]);
        assert!(!partial.complete);
        let full = tracker.complete("thread","turn","patch1","apply_patch",
            &serde_json::json!({"patch":"*** Begin Patch\n*** Update File: missing.txt\n@@\n-old\n+new\n*** End Patch"}).to_string()).unwrap();
        assert!(full.complete);
        assert_eq!(full.changes, partial.changes);
    }
    #[test]
    fn function_channel_and_invalid_inputs_do_not_publish_partial_files() {
        let mut tracker = PatchProgressTracker::default();
        tracker.start("patch1", "apply_patch");
        assert!(tracker.delta("t", "u", "patch1", "{\"patch\":\"").is_none());
        assert!(
            tracker
                .complete("t", "u", "patch1", "apply_patch", "{\"patch\":\"bad\"}")
                .is_none()
        );
    }
}
