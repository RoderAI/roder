// Derived from OpenAI Codex apply-patch at 8b78f4796605bda8e31329537f5fef036e405e66.
// Copyright 2025 OpenAI. Licensed under Apache-2.0; see third-party/codex.
// Adapted for Roder: filesystem-independent parsing and matching.

use std::path::PathBuf;

use super::*;

#[test]
fn test_streaming_patch_parser_streams_complete_lines_before_end_patch() {
    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta("*** Begin Patch\n*** Add File: src/hello.txt\n+hello\n+wor"),
        Ok(vec![AddFile {
            path: PathBuf::from("src/hello.txt"),
            contents: "hello\n".to_string(),
        }])
    );
    assert_eq!(
        parser.push_delta("ld\n"),
        Ok(vec![AddFile {
            path: PathBuf::from("src/hello.txt"),
            contents: "hello\nworld\n".to_string(),
        }])
    );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
            parser.push_delta(
                "*** Begin Patch\n*** Update File: src/old.rs\n*** Move to: src/new.rs\n@@\n-old\n+new\n",
            ),
            Ok(vec![UpdateFile {
                path: PathBuf::from("src/old.rs"),
                move_path: Some(PathBuf::from("src/new.rs")),
                chunks: vec![UpdateFileChunk {
                    change_context: None,
                    old_lines: vec!["old".to_string()],
                    new_lines: vec!["new".to_string()],
                    context_line_indices: vec![],
                    is_end_of_file: false,
                }],
            }])
        );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta("*** Begin Patch\n*** Delete File: gone.txt"),
        Ok(Vec::new())
    );
    assert_eq!(
        parser.push_delta("\n"),
        Ok(vec![DeleteFile {
            path: PathBuf::from("gone.txt"),
        }])
    );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta(
            "*** Begin Patch\n*** Add File: src/one.txt\n+one\n*** Delete File: src/two.txt\n",
        ),
        Ok(vec![
            AddFile {
                path: PathBuf::from("src/one.txt"),
                contents: "one\n".to_string(),
            },
            DeleteFile {
                path: PathBuf::from("src/two.txt"),
            },
        ])
    );
}

#[test]
fn test_streaming_patch_parser_environment_id_mode() {
    let patch = "\
*** Begin Patch
*** Environment ID: remote
*** Add File: src/hello.txt
+hello
*** End Patch
";

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta(patch),
        Ok(vec![AddFile {
            path: PathBuf::from("src/hello.txt"),
            contents: "hello\n".to_string(),
        }])
    );
    assert_eq!(parser.environment_id(), Some("remote"));

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
            parser.push_delta(
                "*** Begin Patch\n*** Environment ID: first\n*** Environment ID: second\n",
            ),
            Err(InvalidPatchError(
                "apply_patch environment_id cannot be specified more than once".to_string(),
            ))
        );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta("*** Begin Patch\n*** Environment ID:   \n"),
        Err(InvalidPatchError(
            "apply_patch environment_id cannot be empty".to_string(),
        ))
    );
}

#[test]
fn test_streaming_patch_parser_large_patch_split_by_character() {
    let patch = "\
*** Begin Patch
*** Add File: docs/release-notes.md
+# Release notes
+
+## CLI
+- Surface apply_patch progress while arguments stream.
+- Keep final patch application gated on the completed tool call.
+- Include file summaries in the progress event payload.
*** Update File: src/config.rs
@@ impl Config
-    pub apply_patch_progress: bool,
+    pub stream_apply_patch_progress: bool,
     pub include_diagnostics: bool,
@@ fn default_progress_interval()
-    Duration::from_millis(500)
+    Duration::from_millis(250)
*** Delete File: src/legacy_patch_progress.rs
*** Update File: crates/cli/src/main.rs
*** Move to: crates/cli/src/bin/codex.rs
@@ fn run()
-    let args = Args::parse();
-    dispatch(args)
+    let cli = Cli::parse();
+    dispatch(cli)
*** Add File: tests/fixtures/apply_patch_progress.json
+{
+  \"type\": \"apply_patch_progress\",
+  \"hunks\": [
+    { \"operation\": \"add\", \"path\": \"docs/release-notes.md\" },
+    { \"operation\": \"update\", \"path\": \"src/config.rs\" }
+  ]
+}
*** Update File: README.md
@@ Development workflow
 Build the Rust workspace before opening a pull request.
+When touching streamed tool calls, include parser coverage for partial input.
+Prefer tests that exercise the exact event payload shape.
*** Delete File: docs/old-apply-patch-progress.md
*** End Patch";

    let mut parser = StreamingPatchParser::default();
    let mut max_hunk_count = 0;
    let mut saw_hunk_counts = Vec::new();
    let mut hunks = Vec::new();
    for ch in patch.chars() {
        let updated_hunks = parser.push_delta(&ch.to_string()).unwrap();
        if !updated_hunks.is_empty() {
            let hunk_count = updated_hunks.len();
            assert!(
                hunk_count >= max_hunk_count,
                "hunk count should never decrease while streaming: {hunk_count} < {max_hunk_count}",
            );
            if hunk_count > max_hunk_count {
                saw_hunk_counts.push(hunk_count);
                max_hunk_count = hunk_count;
            }
            hunks = updated_hunks;
        }
    }

    assert_eq!(saw_hunk_counts, vec![1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(hunks.len(), 7);
    assert_eq!(
        hunks
            .iter()
            .map(|hunk| match hunk {
                AddFile { .. } => "add",
                DeleteFile { .. } => "delete",
                UpdateFile {
                    move_path: Some(_), ..
                } => "move-update",
                UpdateFile {
                    move_path: None, ..
                } => "update",
            })
            .collect::<Vec<_>>(),
        vec![
            "add",
            "update",
            "delete",
            "move-update",
            "add",
            "update",
            "delete"
        ]
    );
}

#[test]
fn test_streaming_patch_parser_keeps_indented_update_markers_as_context_lines() {
    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta(
            "\
*** Begin Patch
*** Update File: a.txt
@@
-old a
+new a
 *** Update File: b.txt
@@
-old b
+new b
*** End Patch
",
        ),
        Ok(vec![UpdateFile {
            path: PathBuf::from("a.txt"),
            move_path: None,
            chunks: vec![
                UpdateFileChunk {
                    change_context: None,
                    old_lines: vec!["old a".to_string(), "*** Update File: b.txt".to_string()],
                    new_lines: vec!["new a".to_string(), "*** Update File: b.txt".to_string()],
                    context_line_indices: vec![(1, 1)],
                    is_end_of_file: false,
                },
                UpdateFileChunk {
                    change_context: None,
                    old_lines: vec!["old b".to_string()],
                    new_lines: vec!["new b".to_string()],
                    context_line_indices: vec![],
                    is_end_of_file: false,
                },
            ],
        }])
    );
}

#[test]
fn test_streaming_patch_parser_preserves_bare_empty_update_lines() {
    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta(
            "\
*** Begin Patch
*** Update File: file.txt
@@
 context before

 context after
*** End Patch
",
        ),
        Ok(vec![UpdateFile {
            path: PathBuf::from("file.txt"),
            move_path: None,
            chunks: vec![UpdateFileChunk {
                change_context: None,
                // The normal parser treats a bare empty line in an update hunk as an
                // empty context line. Preserve that leniency in the streaming parser.
                old_lines: vec![
                    "context before".to_string(),
                    String::new(),
                    "context after".to_string(),
                ],
                new_lines: vec![
                    "context before".to_string(),
                    String::new(),
                    "context after".to_string(),
                ],
                context_line_indices: vec![(0, 0), (1, 1), (2, 2)],
                is_end_of_file: false,
            }],
        }])
    );
}

#[test]
fn test_streaming_patch_parser_ignores_empty_lines_after_end_of_file() {
    let mut parser = StreamingPatchParser::default();
    assert_eq!(
            parser.push_delta(
                "*** Begin Patch\n*** Update File: file.txt\n@@\n+quux\n*** End of File\n\n*** End Patch\n",
            ),
            Ok(vec![UpdateFile {
                path: PathBuf::from("file.txt"),
                move_path: None,
                chunks: vec![UpdateFileChunk {
                    change_context: None,
                    old_lines: Vec::new(),
                    new_lines: vec!["quux".to_string()],
                    context_line_indices: vec![],
                    is_end_of_file: true,
                }],
            }])
        );
}

#[test]
fn test_streaming_patch_parser_matches_line_ending_behavior() {
    let mut parser = StreamingPatchParser::default();
    assert_eq!(
            parser.push_delta("*** Begin Patch\r\n*** Update File: file.txt\r\n@@\r\n-old\r\n+new\r\n*** End Patch\r\n"),
            Ok(vec![UpdateFile {
                path: PathBuf::from("file.txt"),
                move_path: None,
                chunks: vec![UpdateFileChunk {
                    change_context: None,
                    old_lines: vec!["old".to_string()],
                    new_lines: vec!["new".to_string()],
                    context_line_indices: vec![],
                    is_end_of_file: false,
                }],
            }])
        );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
            parser.push_delta("*** Begin Patch\r\n*** Update File: file.txt\r\n@@\r\n-old\r\r\n+new\r\n*** End Patch\r\n"),
            Ok(vec![UpdateFile {
                path: PathBuf::from("file.txt"),
                move_path: None,
                chunks: vec![UpdateFileChunk {
                    change_context: None,
                    old_lines: vec!["old\r".to_string()],
                    new_lines: vec!["new".to_string()],
                    context_line_indices: vec![],
                    is_end_of_file: false,
                }],
            }])
        );
}

#[test]
fn test_streaming_patch_parser_finish_processes_final_line_without_newline() {
    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta("*** Begin Patch\n*** Add File: file.txt\n+hello\n*** End Patch"),
        Ok(vec![AddFile {
            path: PathBuf::from("file.txt"),
            contents: "hello\n".to_string(),
        }])
    );
    assert_eq!(
        parser.finish(),
        Ok(vec![AddFile {
            path: PathBuf::from("file.txt"),
            contents: "hello\n".to_string(),
        }])
    );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta(
            "*** Begin Patch\n*** Update File: file.txt\n@@\n-old\n+new\n *** End Patch",
        ),
        Ok(vec![UpdateFile {
            path: PathBuf::from("file.txt"),
            move_path: None,
            chunks: vec![UpdateFileChunk {
                change_context: None,
                old_lines: vec!["old".to_string()],
                new_lines: vec!["new".to_string()],
                context_line_indices: vec![],
                is_end_of_file: false,
            }],
        }])
    );
    assert_eq!(
        parser.finish(),
        Ok(vec![UpdateFile {
            path: PathBuf::from("file.txt"),
            move_path: None,
            chunks: vec![UpdateFileChunk {
                change_context: None,
                old_lines: vec!["old".to_string()],
                new_lines: vec!["new".to_string()],
                context_line_indices: vec![],
                is_end_of_file: false,
            }],
        }])
    );
}

#[test]
fn test_streaming_patch_parser_finish_requires_end_patch() {
    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta("*** Begin Patch\n*** Add File: file.txt\n+hello\n"),
        Ok(vec![AddFile {
            path: PathBuf::from("file.txt"),
            contents: "hello\n".to_string(),
        }])
    );
    assert_eq!(
        parser.finish(),
        Err(InvalidPatchError(
            "The last line of the patch must be '*** End Patch'".to_string(),
        ))
    );
}

#[test]
fn test_streaming_patch_parser_rejects_content_after_end_patch() {
    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser
            .push_delta("*** Begin Patch\n*** Add File: file.txt\n+hello\n*** End Patch\nextra\n",),
        Err(InvalidPatchError(
            "The last line of the patch must be '*** End Patch'".to_string(),
        ))
    );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta("*** Begin Patch\n*** Add File: file.txt\n+hello\n*** End Patch\n \t\n",),
        Ok(vec![AddFile {
            path: PathBuf::from("file.txt"),
            contents: "hello\n".to_string(),
        }])
    );
}
