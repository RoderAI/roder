use super::*;
use crate::remote_test_support::{RecordingRunnerSession, RecordingRunnerState};
use crate::workspace::ToolPathScope;
use std::sync::Arc;

#[tokio::test]
async fn runner_uses_codex_ordered_context_matching_and_moves() {
    let state = Arc::new(RecordingRunnerState::default());
    state
        .files
        .lock()
        .unwrap()
        .insert("source.txt".into(), b"first\nold\nsecond\nold\n".to_vec());
    state
        .files
        .lock()
        .unwrap()
        .insert("dest.txt".into(), b"overwritten\n".to_vec());
    let session = RecordingRunnerSession {
        state: state.clone(),
    };
    let workspace =
        Workspace::remote("/sandbox/workspace".into(), ToolPathScope::Workspace).unwrap();
    let summary = apply(&workspace, &session, "*** Begin Patch\n*** Update File: source.txt\n*** Move to: dest.txt\n@@ second\n-old\n+new\n*** End Patch").await.unwrap();
    assert_eq!(
        summary.summary,
        "Success. Updated the following files:\nM dest.txt\n"
    );
    let files = state.files.lock().unwrap();
    assert!(!files.contains_key("source.txt"));
    assert_eq!(files["dest.txt"], b"first\nold\nsecond\nnew\n");
}

#[tokio::test]
async fn runner_add_overwrites_and_rejects_invalid_patch_before_io() {
    let state = Arc::new(RecordingRunnerState::default());
    state
        .files
        .lock()
        .unwrap()
        .insert("a.txt".into(), b"old\n".to_vec());
    let session = RecordingRunnerSession {
        state: state.clone(),
    };
    let workspace =
        Workspace::remote("/sandbox/workspace".into(), ToolPathScope::Workspace).unwrap();
    apply(
        &workspace,
        &session,
        "*** Begin Patch\n*** Add File: a.txt\n+new\n*** End Patch",
    )
    .await
    .unwrap();
    assert_eq!(state.files.lock().unwrap()["a.txt"], b"new\n");
    for patch in [
        "*** Begin Patch\n*** Delete File: a.txt\n*** End Patch\ntrailing",
        "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-new\n+changed\n",
        "*** Begin Patch\n*** Add File: ../outside\n+no\n*** End Patch",
    ] {
        assert!(apply(&workspace, &session, patch).await.is_err());
        assert_eq!(state.files.lock().unwrap()["a.txt"], b"new\n");
    }
    assert!(state.commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn runner_preverification_preserves_files_when_later_context_or_path_is_invalid() {
    let state = Arc::new(RecordingRunnerState::default());
    state
        .files
        .lock()
        .unwrap()
        .insert("a".into(), b"original\n".to_vec());
    let session = RecordingRunnerSession {
        state: state.clone(),
    };
    let workspace =
        Workspace::remote("/sandbox/workspace".into(), ToolPathScope::Workspace).unwrap();
    for tail in [
        "*** Update File: missing\n@@\n-old\n+new",
        "*** Update File: ./a\n@@\n-original\n+new",
        "*** Add File: ../outside\n+new",
    ] {
        let patch = format!(
            "*** Begin Patch\n*** Update File: a\n@@\n-original\n+changed\n{tail}\n*** End Patch"
        );
        assert!(apply(&workspace, &session, &patch).await.is_err());
        assert_eq!(state.files.lock().unwrap()["a"], b"original\n");
    }
}
