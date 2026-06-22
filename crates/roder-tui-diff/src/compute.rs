use std::ops::Range;

use similar::{ChangeTag, TextDiff};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum HunkStatus {
    Pending,
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
    Binary,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub before_index: Option<usize>,
    pub after_index: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Hunk {
    pub before_range: Range<usize>,
    pub after_range: Range<usize>,
    pub status: HunkStatus,
    pub lines: Vec<DiffLine>,
}

pub fn compute_diff(before: Option<&str>, after: &str) -> Vec<Hunk> {
    if before.is_some_and(is_binary_text) || is_binary_text(after) {
        return vec![binary_hunk(before, after)];
    }

    let before = normalize_for_diff(before.unwrap_or_default());
    let after = normalize_for_diff(after);
    if before == after {
        return Vec::new();
    }

    let diff = TextDiff::from_lines(&before, &after);
    diff.grouped_ops(3)
        .into_iter()
        .filter_map(|group| {
            let mut lines = Vec::new();
            for op in group {
                for change in diff.iter_changes(&op) {
                    lines.push(DiffLine {
                        kind: match change.tag() {
                            ChangeTag::Equal => DiffLineKind::Context,
                            ChangeTag::Delete => DiffLineKind::Removed,
                            ChangeTag::Insert => DiffLineKind::Added,
                        },
                        before_index: change.old_index(),
                        after_index: change.new_index(),
                        text: change.value().to_string(),
                    });
                }
            }
            (!lines.is_empty()).then(|| hunk_from_lines(lines))
        })
        .collect()
}

fn hunk_from_lines(lines: Vec<DiffLine>) -> Hunk {
    let before_range = line_range(lines.iter().filter_map(|line| line.before_index));
    let after_range = line_range(lines.iter().filter_map(|line| line.after_index));
    Hunk {
        before_range,
        after_range,
        status: HunkStatus::Pending,
        lines,
    }
}

fn line_range(indices: impl Iterator<Item = usize>) -> Range<usize> {
    let mut start = None;
    let mut end = 0;
    for index in indices {
        start = Some(start.map_or(index, |current: usize| current.min(index)));
        end = end.max(index + 1);
    }
    start.unwrap_or(0)..end
}

fn binary_hunk(before: Option<&str>, after: &str) -> Hunk {
    Hunk {
        before_range: 0..before.map(line_count).unwrap_or(0),
        after_range: 0..line_count(after),
        status: HunkStatus::Pending,
        lines: vec![DiffLine {
            kind: DiffLineKind::Binary,
            before_index: None,
            after_index: None,
            text: "Binary file changed".to_string(),
        }],
    }
}

fn is_binary_text(value: &str) -> bool {
    value.as_bytes().contains(&0)
}

fn normalize_for_diff(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
}

fn line_count(value: &str) -> usize {
    normalize_for_diff(value).lines().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_compute_tracks_typical_text_edits() {
        let hunks = compute_diff(Some("one\ntwo\nthree\n"), "one\nTWO\nthree\nfour\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].before_range, 0..3);
        assert_eq!(hunks[0].after_range, 0..4);
        assert!(
            hunks[0]
                .lines
                .iter()
                .any(|line| line.kind == DiffLineKind::Removed && line.text == "two\n")
        );
        assert!(
            hunks[0]
                .lines
                .iter()
                .any(|line| line.kind == DiffLineKind::Added && line.text == "TWO\n")
        );
    }

    #[test]
    fn diff_compute_handles_new_files_and_deletions() {
        let created = compute_diff(None, "hello\n");
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].before_range, 0..0);
        assert_eq!(created[0].after_range, 0..1);
        assert!(
            created[0]
                .lines
                .iter()
                .all(|line| line.kind == DiffLineKind::Added)
        );

        let deleted = compute_diff(Some("hello\n"), "");
        assert_eq!(deleted[0].before_range, 0..1);
        assert_eq!(deleted[0].after_range, 0..0);
        assert!(
            deleted[0]
                .lines
                .iter()
                .all(|line| line.kind == DiffLineKind::Removed)
        );
    }

    #[test]
    fn diff_compute_preserves_trailing_newline_signal() {
        let hunks = compute_diff(Some("hello\n"), "hello");
        assert_eq!(hunks.len(), 1);
        assert!(
            hunks[0]
                .lines
                .iter()
                .any(|line| line.kind == DiffLineKind::Removed && line.text.ends_with('\n'))
        );
        assert!(
            hunks[0]
                .lines
                .iter()
                .any(|line| line.kind == DiffLineKind::Added && !line.text.ends_with('\n'))
        );
    }

    #[test]
    fn diff_compute_handles_unicode_and_normalizes_line_endings_and_tabs() {
        assert!(compute_diff(Some("alpha\r\nbeta\n"), "alpha\nbeta\n").is_empty());
        assert!(compute_diff(Some("a\tb\n"), "a    b\n").is_empty());

        let hunks = compute_diff(Some("hello\n"), "hello\nwaving hand: hi\n");
        assert!(
            hunks[0]
                .lines
                .iter()
                .any(|line| line.kind == DiffLineKind::Added && line.text.contains("waving hand"))
        );
    }

    #[test]
    fn diff_compute_surfaces_binary_inputs() {
        let hunks = compute_diff(Some("a\0b"), "a\0c");
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].lines[0].kind, DiffLineKind::Binary);
        assert_eq!(hunks[0].lines[0].text, "Binary file changed");
    }
}
