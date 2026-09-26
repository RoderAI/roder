// Derived from OpenAI Codex apply-patch at 8b78f4796605bda8e31329537f5fef036e405e66.
// Copyright 2025 OpenAI. Licensed under Apache-2.0; see third-party/codex.
// Adapted for Roder: filesystem-independent parsing and matching.

use super::*;

#[test]
fn test_streaming_patch_parser_returns_errors() {
    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta("bad\n"),
        Err(InvalidPatchError(
            "The first line of the patch must be '*** Begin Patch'".to_string(),
        ))
    );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(parser.push_delta("*** Begin Patch\n"), Ok(Vec::new()));
    assert_eq!(
            parser.push_delta("bad\n"),
            Err(InvalidHunkError {
                message: "'bad' is not a valid hunk header. Valid hunk headers: '*** Add File: {path}', '*** Delete File: {path}', '*** Update File: {path}'"
                    .to_string(),
                line_number: 2,
            })
        );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
            parser.push_delta("*** Begin Patch\n*** Add File: file.txt\nbad\n"),
            Err(InvalidHunkError {
                message: "'bad' is not a valid hunk header. Valid hunk headers: '*** Add File: {path}', '*** Delete File: {path}', '*** Update File: {path}'"
                    .to_string(),
                line_number: 3,
            })
        );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
            parser.push_delta("*** Begin Patch\n*** Delete File: file.txt\nbad\n"),
            Err(InvalidHunkError {
                message: "'bad' is not a valid hunk header. Valid hunk headers: '*** Add File: {path}', '*** Delete File: {path}', '*** Update File: {path}'"
                    .to_string(),
                line_number: 3,
            })
        );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta("*** Begin Patch\n*** Update File: file.txt\n*** End Patch\n"),
        Err(InvalidHunkError {
            message: "Update file hunk for path 'file.txt' is empty".to_string(),
            line_number: 2,
        })
    );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
            parser.push_delta(
                "*** Begin Patch\n*** Update File: old.txt\n*** Move to: new.txt\n*** Delete File: other.txt\n",
            ),
            Err(InvalidHunkError {
                message: "Update file hunk for path 'old.txt' is empty".to_string(),
                line_number: 2,
            })
        );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta("*** Begin Patch\n*** Update File: file.txt\n@@\n*** End Patch\n"),
        Err(InvalidHunkError {
            message: "Update hunk does not contain any lines".to_string(),
            line_number: 4,
        })
    );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta("*** Begin Patch\n*** Update File: file.txt\n@@\n*** End of File\n"),
        Err(InvalidHunkError {
            message: "Update hunk does not contain any lines".to_string(),
            line_number: 4,
        })
    );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
            parser.push_delta("*** Begin Patch\n*** Update File: file.txt\n@@\n@@\n"),
            Err(InvalidHunkError {
                message: "Unexpected line found in update hunk: '@@'. Every line should start with ' ' (context line), '+' (added line), or '-' (removed line)"
                    .to_string(),
                line_number: 4,
            })
        );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
        parser.push_delta("*** Begin Patch\n*** Update File: file.txt\n@@\n-old\nbad\n"),
        Err(InvalidHunkError {
            message: "Expected update hunk to start with a @@ context marker, got: 'bad'"
                .to_string(),
            line_number: 5,
        })
    );

    let mut parser = StreamingPatchParser::default();
    assert_eq!(
            parser.push_delta(
                "*** Begin Patch\n*** Update File: file.txt\n@@\n*** Update File: other.txt\n",
            ),
            Err(InvalidHunkError {
                message: "Unexpected line found in update hunk: '*** Update File: other.txt'. Every line should start with ' ' (context line), '+' (added line), or '-' (removed line)"
                    .to_string(),
                line_number: 4,
            })
        );
}
