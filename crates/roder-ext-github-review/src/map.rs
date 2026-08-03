//! Finding → pull-request review comment mapping.
//!
//! GitHub rejects the **entire** review with a 422 when a single comment points
//! at a path or line that is not part of the pull request's diff, so every
//! comment is validated against the diff hunks of `pulls/{n}/files` before the
//! request is built. Anything that cannot be anchored is moved into the summary
//! body and reported back as a [`ReviewPublishSkip`] instead of being dropped.
//!
//! The commentable set is the full NEW-file span of each hunk — context lines
//! included, not just added lines — which is what the API actually accepts.

use std::path::{Component, Path};

use roder_api::review::{ReviewFinding, ReviewOutput, ReviewPriority, ReviewPublishSkip};
use serde::Deserialize;
use serde_json::{Value, json};

/// GitHub truncates very long comment bodies; keep well under the limit.
const MAX_COMMENT_CHARS: usize = 60_000;

/// A file entry from `GET /repos/{owner}/{repo}/pulls/{number}/files`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PullRequestFile {
    pub filename: String,
    /// Absent for binary files and diffs GitHub considers too large; such
    /// files have no commentable lines at all.
    #[serde(default)]
    pub patch: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

/// Inclusive NEW-file line span of one diff hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkSpan {
    pub start: u32,
    pub end: u32,
}

impl HunkSpan {
    pub fn contains(self, line: u32) -> bool {
        line >= self.start && line <= self.end
    }
}

/// Parses the NEW-file spans out of a unified-diff `patch`.
///
/// A `+start,0` hunk adds nothing to the new file and therefore contributes no
/// commentable lines; `+start` without a count means exactly one line.
pub fn parse_hunk_spans(patch: &str) -> Vec<HunkSpan> {
    patch.lines().filter_map(parse_hunk_header).collect()
}

fn parse_hunk_header(line: &str) -> Option<HunkSpan> {
    let rest = line.strip_prefix("@@")?;
    let (ranges, _) = rest.split_once("@@")?;
    let added = ranges
        .split_whitespace()
        .find_map(|part| part.strip_prefix('+'))?;
    let (start, count) = match added.split_once(',') {
        Some((start, count)) => (start.parse::<u32>().ok()?, count.parse::<u32>().ok()?),
        None => (added.parse::<u32>().ok()?, 1),
    };
    if count == 0 || start == 0 {
        return None;
    }
    Some(HunkSpan {
        start,
        end: start + count - 1,
    })
}

pub struct PayloadOptions<'a> {
    /// The pull request's `headRefOid`; anything else is rejected.
    pub commit_id: &'a str,
    /// Root the finding paths are relativized against — the git repo root, not
    /// the workspace root, because worktree forks make the two differ.
    pub repo_root: &'a Path,
    pub files: &'a [PullRequestFile],
    pub event: &'a str,
    /// Least severe priority still published; less severe findings are skipped
    /// silently rather than appended to the summary.
    pub min_priority: ReviewPriority,
}

#[derive(Debug, Clone)]
pub struct MappedReview {
    /// Body of `POST /repos/{owner}/{repo}/pulls/{number}/reviews`.
    pub payload: Value,
    pub comment_count: usize,
    pub skipped: Vec<ReviewPublishSkip>,
}

enum Mapped {
    Comment(Value),
    /// Cannot be anchored in the diff — goes into the summary body.
    Unanchored(String),
    /// Deliberately dropped by configuration — mentioned nowhere.
    Filtered(String),
}

pub fn build_review_payload(
    output: &ReviewOutput,
    selected: Option<&[usize]>,
    options: &PayloadOptions<'_>,
) -> MappedReview {
    let indexes: Vec<usize> = match selected {
        Some(selected) => selected
            .iter()
            .copied()
            .filter(|index| *index < output.findings.len())
            .collect(),
        None => (0..output.findings.len()).collect(),
    };

    let mut comments = Vec::new();
    let mut unanchored = Vec::new();
    let mut skipped = Vec::new();
    for index in indexes {
        let finding = &output.findings[index];
        match map_finding(finding, options) {
            Mapped::Comment(comment) => comments.push(comment),
            Mapped::Unanchored(reason) => {
                unanchored.push((finding, reason.clone()));
                skipped.push(ReviewPublishSkip {
                    finding_index: index,
                    reason,
                });
            }
            Mapped::Filtered(reason) => skipped.push(ReviewPublishSkip {
                finding_index: index,
                reason,
            }),
        }
    }

    let body = render_summary(output, &unanchored, options.repo_root);
    let comment_count = comments.len();
    MappedReview {
        payload: json!({
            "commit_id": options.commit_id,
            "event": options.event,
            "body": body,
            "comments": comments,
        }),
        comment_count,
        skipped,
    }
}

fn map_finding(finding: &ReviewFinding, options: &PayloadOptions<'_>) -> Mapped {
    if finding.priority > options.min_priority {
        return Mapped::Filtered(format!(
            "priority {} is below the configured minimum {}",
            finding.priority, options.min_priority
        ));
    }

    let location = &finding.code_location;
    let Some(path) = relative_path(&location.absolute_file_path, options.repo_root) else {
        return Mapped::Unanchored(format!(
            "{} is outside the repository at {}",
            location.absolute_file_path.display(),
            options.repo_root.display()
        ));
    };

    let (start, end) = normalized_range(finding);
    if end == 0 {
        return Mapped::Unanchored(format!("{path} has no usable line number"));
    }

    let Some(file) = options.files.iter().find(|file| file.filename == path) else {
        return Mapped::Unanchored(format!("{path} is not part of the pull request diff"));
    };
    let Some(patch) = file.patch.as_deref() else {
        return Mapped::Unanchored(format!(
            "{path} has no diff hunks ({})",
            file.status.as_deref().unwrap_or("binary or too large")
        ));
    };

    let spans = parse_hunk_spans(patch);
    let Some(span) = spans.iter().copied().find(|span| span.contains(end)) else {
        return Mapped::Unanchored(format!(
            "{path}:{end} is outside the pull request diff hunks"
        ));
    };

    let mut comment = json!({
        "path": path,
        "side": "RIGHT",
        "line": end,
        "body": comment_body(finding),
    });
    // `start_line` is only legal below `line`, and both ends must sit inside
    // the same hunk; otherwise degrade to a single-line comment rather than
    // losing the finding to a 422.
    if start < end && span.contains(start) {
        comment["start_line"] = json!(start);
        comment["start_side"] = json!("RIGHT");
    }
    Mapped::Comment(comment)
}

/// Model-reported ranges are occasionally inverted; order them instead of
/// letting GitHub reject the whole review.
fn normalized_range(finding: &ReviewFinding) -> (u32, u32) {
    let range = finding.code_location.line_range;
    if range.start > range.end {
        (range.end, range.start)
    } else {
        (range.start, range.end)
    }
}

/// Repo-root-relative, forward-slashed, no `./` prefix. Paths that escape the
/// repo root (or are not absolute and not already relative to it) are rejected.
pub fn relative_path(path: &Path, repo_root: &Path) -> Option<String> {
    let relative = if path.is_absolute() {
        path.strip_prefix(repo_root).ok()?
    } else {
        path
    };
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn comment_body(finding: &ReviewFinding) -> String {
    let mut body = format!(
        "**[{}] {}**\n\n{}",
        finding.priority.to_string().to_uppercase(),
        finding.display_title(),
        finding.body.trim()
    );
    body.push_str("\n\n_");
    if let Some(confidence) = finding.confidence_score {
        body.push_str(&format!("confidence {confidence:.2} · "));
    }
    body.push_str("reported by Roder_");
    truncate(body, MAX_COMMENT_CHARS)
}

fn render_summary(
    output: &ReviewOutput,
    unanchored: &[(&ReviewFinding, String)],
    repo_root: &Path,
) -> String {
    let mut body = String::from("## Roder review\n");
    if let Some(explanation) = output.overall_explanation.as_deref().map(str::trim)
        && !explanation.is_empty()
    {
        body.push('\n');
        body.push_str(explanation);
        body.push('\n');
    }

    let mut facts = Vec::new();
    if let Some(correctness) = output.overall_correctness.as_deref().map(str::trim)
        && !correctness.is_empty()
    {
        facts.push(format!("**Correctness:** {correctness}"));
    }
    if let Some(confidence) = output.overall_confidence_score {
        facts.push(format!("**Confidence:** {confidence:.2}"));
    }
    if !facts.is_empty() {
        body.push('\n');
        body.push_str(&facts.join("  •  "));
        body.push('\n');
    }

    if !unanchored.is_empty() {
        body.push_str("\n### Findings outside this diff\n\n");
        for (finding, reason) in unanchored {
            let location = &finding.code_location;
            let path = relative_path(&location.absolute_file_path, repo_root)
                .unwrap_or_else(|| location.absolute_file_path.display().to_string());
            body.push_str(&format!(
                "- **[{}] {}** — `{path}:{}-{}` ({reason})\n\n  {}\n",
                finding.priority.to_string().to_uppercase(),
                finding.display_title(),
                location.line_range.start,
                location.line_range.end,
                finding.body.trim().replace('\n', "\n  ")
            ));
        }
    }
    truncate(body, MAX_COMMENT_CHARS)
}

fn truncate(mut text: String, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text;
    }
    let cut = text
        .char_indices()
        .nth(limit.saturating_sub(1))
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    text.truncate(cut);
    text.push('…');
    text
}

#[cfg(test)]
#[path = "map_tests.rs"]
mod tests;
