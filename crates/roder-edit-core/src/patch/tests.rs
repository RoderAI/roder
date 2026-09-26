use super::*;
use std::path::PathBuf;

#[test]
fn parses_and_applies_codex_patch() {
    let root = temp_dir("roder-edit-core-patch");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "old\n").unwrap();
    let output = apply_codex_patch_to_workspace(
        &root,
        "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** End Patch\n",
    )
    .unwrap();
    assert!(output.summary.contains("M a.txt"));
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "new\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rejects_paths_outside_workspace() {
    let root = temp_dir("roder-edit-core-outside");
    std::fs::create_dir_all(&root).unwrap();
    let err = apply_codex_patch_to_workspace(
        &root,
        "*** Begin Patch\n*** Add File: ../x.txt\n+no\n*** End Patch\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("outside workspace"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn can_allow_paths_outside_workspace() {
    let root = temp_dir("roder-edit-core-allow-root");
    let outside = temp_dir("roder-edit-core-allow-outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let target = outside.join("x.txt");
    let output = apply_codex_patch_to_workspace_with_external_paths(
        &root,
        &format!(
            "*** Begin Patch\n*** Add File: {}\n+yes\n*** End Patch\n",
            target.display()
        ),
        true,
    )
    .unwrap();

    assert!(
        output
            .summary
            .contains("Success. Updated the following files:")
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "yes\n");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(outside);
}

fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}

// Expected bytes derived from Codex file_update/seek_sequence regression cases.
#[test]
fn ordered_matching_codex_fixtures() {
    for (original, body, expected) in [
        (
            "first\nold\nsecond\nold\n",
            "@@ second\n-old\n+new",
            "first\nold\nsecond\nnew\n",
        ),
        ("old\nold\n", "@@\n-old\n+one\n@@\n-old\n+two", "one\ntwo\n"),
        (
            "old\nold\n",
            "@@\n-old\n+new\n*** End of File",
            "old\nnew\n",
        ),
        ("before\n", "@@\n+after", "before\nafter\n"),
        ("before", "@@\n-before\n+after", "after\n"),
        ("  old \t\n", "@@\n-old\n+new", "new\n"),
        (
            "intro\nsmart—dash ’quote’\n",
            "@@\n-smart-dash 'quote'\n+plain",
            "intro\nplain\n",
        ),
        ("first\n\nlast\n", " first\n\n-last\n+new", "first\n\nnew\n"),
        ("old\n", "@@\n-old", ""),
        ("", "@@\n+new", "new\n"),
        ("a\n\n", "@@\n-a\n-\n+b\n+", "b\n"),
        ("old\r\nkeep\r\n", "@@\n-old\n+new", "new\nkeep\r\n"),
    ] {
        let parsed = parse_patch(&format!(
            "*** Begin Patch\n*** Update File: fixture.txt\n{body}\n*** End Patch"
        ))
        .unwrap();
        let Hunk::UpdateFile { chunks, .. } = &parsed.hunks[0] else {
            panic!()
        };
        assert_eq!(
            derive_new_contents(
                original,
                "fixture.txt",
                chunks,
                ApplyPatchFileUpdateMode::default()
            )
            .unwrap(),
            expected,
            "{body}"
        );
    }
}

#[test]
fn preserves_context_bytes_and_mixed_line_endings_when_selected() {
    let parsed = parse_patch(
        "*** Begin Patch\n*** Update File: file\n@@\n first\n-old\n+new\n last\n*** End Patch",
    )
    .unwrap();
    let Hunk::UpdateFile { chunks, .. } = &parsed.hunks[0] else {
        panic!()
    };
    assert_eq!(
        derive_new_contents(
            "first \t\r\nold\nlast\r",
            "file",
            chunks,
            ApplyPatchFileUpdateMode::PreserveLineEndings
        )
        .unwrap(),
        "first \t\r\nnew\r\nlast\r"
    );
}

#[test]
fn grammar_errors_and_line_numbers_match_codex() {
    for (patch, expected) in [
        (
            "bad",
            "invalid patch: The first line of the patch must be '*** Begin Patch'",
        ),
        (
            "*** Begin Patch\nbad",
            "invalid patch: The last line of the patch must be '*** End Patch'",
        ),
        (
            "*** Begin Patch\n*** Update File: f\n*** End Patch",
            "invalid hunk at line 2, Update file hunk for path 'f' is empty",
        ),
        (
            "*** Begin Patch\n*** Update File: f\n@@\n*** End Patch",
            "invalid hunk at line 4, Update hunk does not contain any lines",
        ),
        (
            "*** Begin Patch\n*** End Patch\ntrailing",
            "invalid patch: The last line of the patch must be '*** End Patch'",
        ),
    ] {
        assert_eq!(parse_patch(patch).unwrap_err().to_string(), expected);
    }
    assert!(
        parse_patch("<<'EOF'\n*** Begin Patch\n*** Add File: f\n+yes\n*** End Patch\nEOF\n")
            .is_ok()
    );
}

#[test]
fn add_overwrite_move_delete_and_summary_match_codex() {
    let root = temp_dir("roder-codex-operations");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a"), "old\n").unwrap();
    std::fs::write(root.join("b"), "destination\n").unwrap();
    std::fs::write(root.join("gone"), "delete\n").unwrap();
    let output = apply_codex_patch_to_workspace(&root, "*** Begin Patch\n*** Delete File: gone\n*** Update File: a\n*** Move to: b\n@@\n-old\n+moved\n*** Add File: nested/added\n+added\n*** Add File: b\n+overwritten\n*** End Patch").unwrap();
    assert_eq!(
        output.summary,
        "Success. Updated the following files:\nA nested/added\nA b\nM b\nD gone\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("b")).unwrap(),
        "overwritten\n"
    );
    assert!(!root.join("a").exists());
    assert!(!root.join("gone").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn missing_parents_cannot_hide_an_external_symlink() {
    let root = temp_dir("roder-patch-symlink");
    let outside = temp_dir("roder-patch-symlink-outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
    let err = apply_codex_patch_to_workspace(
        &root,
        "*** Begin Patch\n*** Add File: link/missing/nested/file\n+no\n*** End Patch",
    )
    .unwrap_err();
    assert!(err.to_string().contains("outside workspace"));
    assert!(!outside.join("missing").exists());
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

#[test]
fn verifies_all_hunks_and_rejects_duplicate_sources_before_any_write() {
    let root = temp_dir("roder-preverified-patch");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a"), "original\n").unwrap();
    for tail in [
        "*** Update File: missing\n@@\n-old\n+new",
        "*** Update File: ./a\n@@\n-original\n+new",
        "*** Add File: ../outside\n+new",
    ] {
        let patch = format!(
            "*** Begin Patch\n*** Update File: a\n@@\n-original\n+changed\n{tail}\n*** End Patch"
        );
        assert!(apply_codex_patch_to_workspace(&root, &patch).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("a")).unwrap(),
            "original\n"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn result_hunks_report_actual_fuzzy_matches_and_file_line_numbers() {
    let root = temp_dir("roder-actual-patch-diff");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a"), "before\n  old \t\nafter\n").unwrap();
    let outcome = apply_codex_patch_to_workspace(
        &root,
        "*** Begin Patch\n*** Update File: a\n@@\n-old\n+new\n*** End Patch",
    )
    .unwrap();
    assert!(outcome.exact);
    let hunk = &outcome.hunks()[0];
    let removed = hunk
        .diff
        .iter()
        .find(|line| line.kind == crate::hunks::HunkDiffLineKind::Removed)
        .unwrap();
    assert_eq!(removed.text, "  old \t");
    assert_eq!(removed.old_line, Some(2));
    assert!(hunk.reverse_patch.as_ref().unwrap().contains("+  old \t"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failures_report_only_applied_changes_and_uncertainty() {
    let root = temp_dir("roder-partial-patch");
    std::fs::create_dir_all(root.join("directory")).unwrap();
    let error = apply_codex_patch_to_workspace(&root, "*** Begin Patch\n*** Add File: applied\n+yes\n*** Add File: directory\n+not-a-file\n*** End Patch").unwrap_err();
    let failure = error.downcast_ref::<PatchFailure>().unwrap();
    assert_eq!(failure.partial.changes.len(), 1);
    assert_eq!(failure.partial.changes[0].path, "applied");
    assert!(!failure.partial.exact);
    assert!(failure.partial.hunks()[0].reverse_patch.is_none());
    assert_eq!(
        std::fs::read_to_string(root.join("applied")).unwrap(),
        "yes\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn deleting_a_symlink_unlinks_it_and_preserves_the_target() {
    let root = temp_dir("roder-patch-unlink");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("target"), "unchanged\n").unwrap();
    std::os::unix::fs::symlink("target", root.join("link")).unwrap();
    let outcome = apply_codex_patch_to_workspace(
        &root,
        "*** Begin Patch\n*** Delete File: link\n*** End Patch",
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("target")).unwrap(),
        "unchanged\n"
    );
    assert!(!root.join("link").exists());
    assert!(
        !outcome.exact,
        "symlink metadata cannot be reconstructed from a text-only delta"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn workspace_scope_rejects_dangling_external_symlinks_before_creating_targets() {
    let root = temp_dir("roder-patch-dangling");
    let outside = temp_dir("roder-patch-dangling-outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(outside.join("new"), root.join("link")).unwrap();
    assert!(
        apply_codex_patch_to_workspace(
            &root,
            "*** Begin Patch\n*** Add File: link\n+no\n*** End Patch"
        )
        .is_err()
    );
    assert!(!outside.join("new").exists());
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}
