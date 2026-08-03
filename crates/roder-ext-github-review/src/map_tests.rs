//! Mapping tests. Everything here is offline: the only GitHub input is a
//! captured `pulls/{n}/files` patch fixture.

use std::path::{Path, PathBuf};

use roder_api::review::{
    ReviewCodeLocation, ReviewFinding, ReviewLineRange, ReviewOutput, ReviewPriority,
};

use super::*;

/// Real header shape from the fixture PR used to verify the API contract:
/// one hunk covering NEW-file lines 6..=30.
const FIXTURE_PATCH: &str = "@@ -6,9 +6,25 @@ def main():\n context\n+added\n context\n";

fn repo_root() -> PathBuf {
    PathBuf::from("/repo")
}

fn finding(path: &str, start: u32, end: u32) -> ReviewFinding {
    ReviewFinding {
        title: "Off-by-one".to_string(),
        body: "The loop overruns the slice.".to_string(),
        confidence_score: Some(0.74),
        priority: ReviewPriority::P1,
        code_location: ReviewCodeLocation {
            absolute_file_path: PathBuf::from(path),
            line_range: ReviewLineRange { start, end },
        },
    }
}

fn files(patch: Option<&str>) -> Vec<PullRequestFile> {
    vec![PullRequestFile {
        filename: "src/main.py".to_string(),
        patch: patch.map(str::to_string),
        status: Some("modified".to_string()),
    }]
}

fn options<'a>(files: &'a [PullRequestFile], root: &'a Path) -> PayloadOptions<'a> {
    PayloadOptions {
        commit_id: "deadbeef",
        repo_root: root,
        files,
        event: "COMMENT",
        min_priority: ReviewPriority::P3,
    }
}

fn map_one(finding: ReviewFinding, files: &[PullRequestFile]) -> MappedReview {
    let root = repo_root();
    let output = ReviewOutput {
        findings: vec![finding],
        ..ReviewOutput::default()
    };
    build_review_payload(&output, None, &options(files, &root))
}

#[test]
fn hunk_headers_parse_into_new_file_spans() {
    let cases: &[(&str, Vec<HunkSpan>)] = &[
        // start,count -> inclusive span
        (
            "@@ -6,9 +6,25 @@ def main():",
            vec![HunkSpan { start: 6, end: 30 }],
        ),
        // omitted count means exactly one line
        ("@@ -1 +42 @@", vec![HunkSpan { start: 42, end: 42 }]),
        // pure deletion adds nothing to the new file
        ("@@ -10,4 +9,0 @@", Vec::new()),
        // several hunks in one patch
        (
            "@@ -1,2 +1,2 @@\n ctx\n@@ -20,0 +21,3 @@\n+a\n+b\n+c\n",
            vec![
                HunkSpan { start: 1, end: 2 },
                HunkSpan { start: 21, end: 23 },
            ],
        ),
        // body lines that merely start with '+' are not headers
        ("+@@ not a header @@", Vec::new()),
        ("@@ malformed @@", Vec::new()),
        ("", Vec::new()),
    ];
    for (patch, expected) in cases {
        assert_eq!(&parse_hunk_spans(patch), expected, "patch: {patch:?}");
    }
}

#[test]
fn paths_are_relativized_to_the_repo_root() {
    let root = Path::new("/repo");
    let cases: &[(&str, Option<&str>)] = &[
        ("/repo/src/main.py", Some("src/main.py")),
        ("/repo/./src/main.py", Some("src/main.py")),
        ("src/main.py", Some("src/main.py")),
        ("./src/main.py", Some("src/main.py")),
        ("/elsewhere/src/main.py", None),
        ("/repo", None),
        ("../escape.py", None),
    ];
    for (input, expected) in cases {
        assert_eq!(
            relative_path(Path::new(input), root).as_deref(),
            *expected,
            "input: {input}"
        );
    }
}

#[test]
fn a_line_inside_a_hunk_becomes_a_single_line_comment() {
    let mapped = map_one(
        finding("/repo/src/main.py", 9, 9),
        &files(Some(FIXTURE_PATCH)),
    );
    assert_eq!(mapped.comment_count, 1);
    assert!(mapped.skipped.is_empty());
    let comment = &mapped.payload["comments"][0];
    assert_eq!(comment["path"], "src/main.py");
    assert_eq!(comment["line"], 9);
    assert_eq!(comment["side"], "RIGHT");
    assert!(
        comment.get("start_line").is_none(),
        "start == end must not emit start_line"
    );
    assert!(
        comment["body"]
            .as_str()
            .expect("body")
            .starts_with("**[P1] Off-by-one**")
    );
    assert_eq!(mapped.payload["commit_id"], "deadbeef");
    assert_eq!(mapped.payload["event"], "COMMENT");
}

#[test]
fn a_multi_line_range_inside_one_hunk_emits_start_line() {
    let mapped = map_one(
        finding("/repo/src/main.py", 16, 20),
        &files(Some(FIXTURE_PATCH)),
    );
    let comment = &mapped.payload["comments"][0];
    assert_eq!(comment["start_line"], 16);
    assert_eq!(comment["start_side"], "RIGHT");
    assert_eq!(comment["line"], 20);
}

#[test]
fn an_inverted_range_is_ordered_instead_of_rejected() {
    let mapped = map_one(
        finding("/repo/src/main.py", 20, 16),
        &files(Some(FIXTURE_PATCH)),
    );
    let comment = &mapped.payload["comments"][0];
    assert_eq!(comment["start_line"], 16);
    assert_eq!(comment["line"], 20);
}

#[test]
fn a_range_starting_outside_the_hunk_degrades_to_one_line() {
    // 30 is the last line of the hunk; 2 is outside it.
    let mapped = map_one(
        finding("/repo/src/main.py", 2, 30),
        &files(Some(FIXTURE_PATCH)),
    );
    let comment = &mapped.payload["comments"][0];
    assert_eq!(comment["line"], 30);
    assert!(comment.get("start_line").is_none());
    assert!(mapped.skipped.is_empty());
}

#[test]
fn unmappable_findings_move_into_the_summary_and_are_reported() {
    let cases: &[(&str, u32, u32, Option<&str>, &str)] = &[
        // one past the end of the hunk (verified 422)
        ("/repo/src/main.py", 31, 31, Some(FIXTURE_PATCH), "outside"),
        // before the hunk (verified 422)
        ("/repo/src/main.py", 2, 2, Some(FIXTURE_PATCH), "outside"),
        // path is not in the PR at all (verified 422)
        (
            "/repo/src/other.py",
            9,
            9,
            Some(FIXTURE_PATCH),
            "not part of the pull request diff",
        ),
        // binary/too-large file: no patch, so no commentable lines
        ("/repo/src/main.py", 9, 9, None, "no diff hunks"),
        // outside the repository entirely
        (
            "/elsewhere/src/main.py",
            9,
            9,
            Some(FIXTURE_PATCH),
            "outside the repository",
        ),
        // no line information at all
        (
            "/repo/src/main.py",
            0,
            0,
            Some(FIXTURE_PATCH),
            "no usable line",
        ),
    ];
    for (path, start, end, patch, expected_reason) in cases {
        let mapped = map_one(finding(path, *start, *end), &files(*patch));
        assert_eq!(
            mapped.comment_count, 0,
            "{path}:{start}-{end} must not become a comment"
        );
        assert_eq!(mapped.skipped.len(), 1);
        assert_eq!(mapped.skipped[0].finding_index, 0);
        assert!(
            mapped.skipped[0].reason.contains(expected_reason),
            "reason {:?} should mention {expected_reason}",
            mapped.skipped[0].reason
        );
        let body = mapped.payload["body"].as_str().expect("body");
        assert!(
            body.contains("Findings outside this diff"),
            "unanchored findings must survive in the summary"
        );
        assert!(body.contains("Off-by-one"));
    }
}

#[test]
fn one_bad_finding_does_not_take_the_good_ones_with_it() {
    let root = repo_root();
    let files = files(Some(FIXTURE_PATCH));
    let output = ReviewOutput {
        findings: vec![
            finding("/repo/src/main.py", 9, 9),
            finding("/repo/src/main.py", 999, 999),
            finding("/repo/src/main.py", 16, 20),
        ],
        overall_explanation: Some("Two real problems.".to_string()),
        overall_correctness: Some("patch is incorrect".to_string()),
        overall_confidence_score: Some(0.82),
    };
    let mapped = build_review_payload(&output, None, &options(&files, &root));
    assert_eq!(mapped.comment_count, 2);
    assert_eq!(mapped.skipped.len(), 1);
    assert_eq!(mapped.skipped[0].finding_index, 1);
    let body = mapped.payload["body"].as_str().expect("body");
    assert!(body.starts_with("## Roder review"));
    assert!(body.contains("Two real problems."));
    assert!(body.contains("**Correctness:** patch is incorrect"));
    assert!(body.contains("**Confidence:** 0.82"));
}

#[test]
fn selection_limits_the_published_findings() {
    let root = repo_root();
    let files = files(Some(FIXTURE_PATCH));
    let output = ReviewOutput {
        findings: vec![
            finding("/repo/src/main.py", 9, 9),
            finding("/repo/src/main.py", 16, 20),
        ],
        ..ReviewOutput::default()
    };
    let mapped = build_review_payload(&output, Some(&[1]), &options(&files, &root));
    assert_eq!(mapped.comment_count, 1);
    assert_eq!(mapped.payload["comments"][0]["line"], 20);
    // Out-of-range indexes are ignored rather than panicking.
    let mapped = build_review_payload(&output, Some(&[7]), &options(&files, &root));
    assert_eq!(mapped.comment_count, 0);
    assert!(mapped.skipped.is_empty());
}

#[test]
fn min_priority_drops_noise_without_mentioning_it_in_the_summary() {
    let root = repo_root();
    let files = files(Some(FIXTURE_PATCH));
    let mut low = finding("/repo/src/main.py", 9, 9);
    low.priority = ReviewPriority::P3;
    low.title = "Nit".to_string();
    let output = ReviewOutput {
        findings: vec![low],
        ..ReviewOutput::default()
    };
    let mut options = options(&files, &root);
    options.min_priority = ReviewPriority::P2;
    let mapped = build_review_payload(&output, None, &options);
    assert_eq!(mapped.comment_count, 0);
    assert_eq!(mapped.skipped.len(), 1);
    assert!(mapped.skipped[0].reason.contains("below the configured"));
    let body = mapped.payload["body"].as_str().expect("body");
    assert!(!body.contains("Nit"));
}

#[test]
fn comment_bodies_are_truncated() {
    let mut long = finding("/repo/src/main.py", 9, 9);
    long.body = "x".repeat(MAX_COMMENT_CHARS * 2);
    let mapped = map_one(long, &files(Some(FIXTURE_PATCH)));
    let body = mapped.payload["comments"][0]["body"]
        .as_str()
        .expect("body");
    assert_eq!(body.chars().count(), MAX_COMMENT_CHARS);
    assert!(body.ends_with('…'));
}
