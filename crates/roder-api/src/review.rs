//! Read-only code review.
//!
//! A **review** is a detached, read-only sub-turn that inspects a diff and
//! emits structured [`ReviewFinding`]s instead of editing the workspace. The
//! findings are surfaced in the TUI and can optionally be handed to a
//! [`ReviewPublisher`] — a pluggable extension service that submits them to an
//! external platform (GitHub pull-request reviews, an issue tracker, chat).
//!
//! Core code only sees these types: publishers are registered on the extension
//! registry like any other service and resolved by id.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub type ReviewId = String;
pub type ReviewPublisherId = String;

/// What the reviewer should look at. Resolved against the workspace's version
/// control provider to produce a [`ReviewRequest`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ReviewTarget {
    /// Everything not yet committed in the workspace.
    #[default]
    UncommittedChanges,
    /// Everything on the current line since it diverged from `branch`.
    BaseBranch { branch: String },
    /// A single commit.
    Commit {
        sha: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// Free-form scope described in natural language.
    Custom { instructions: String },
}

/// Resolved, model-facing review request: the product of a [`ReviewTarget`]
/// and the current version-control state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub target: ReviewTarget,
    /// Human-readable one-liner shown in the TUI ("review vs main").
    pub label: String,
    /// Natural-language prompt handed to the review turn.
    pub prompt: String,
    /// Merge-base revision when the target implies one; used by publishers to
    /// anchor comments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    pub workspace_root: PathBuf,
}

/// Whether the caller waits for the review turn or is notified later.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReviewDelivery {
    /// Await the review turn and return its findings to the caller.
    #[default]
    Inline,
    /// Start the review turn and return immediately; findings arrive as
    /// events.
    Detached,
}

/// Severity of a finding. `P0` is the most severe; `P2` is the default when
/// the model omits one.
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPriority {
    P0,
    P1,
    #[default]
    P2,
    P3,
}

impl ReviewPriority {
    pub const ALL: [ReviewPriority; 4] = [
        ReviewPriority::P0,
        ReviewPriority::P1,
        ReviewPriority::P2,
        ReviewPriority::P3,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ReviewPriority::P0 => "p0",
            ReviewPriority::P1 => "p1",
            ReviewPriority::P2 => "p2",
            ReviewPriority::P3 => "p3",
        }
    }

    /// Case-insensitive parse of `p0`..`p3`; `None` for anything else so
    /// callers can fall back to [`ReviewPriority::default`].
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "p0" => Some(ReviewPriority::P0),
            "p1" => Some(ReviewPriority::P1),
            "p2" => Some(ReviewPriority::P2),
            "p3" => Some(ReviewPriority::P3),
            _ => None,
        }
    }
}

impl std::fmt::Display for ReviewPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReviewPriority {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
            .ok_or_else(|| format!("unknown review priority {value}; expected p0..p3"))
    }
}

/// Inclusive 1-based line range inside a file.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewLineRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCodeLocation {
    pub absolute_file_path: PathBuf,
    pub line_range: ReviewLineRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFinding {
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_score: Option<f32>,
    #[serde(default)]
    pub priority: ReviewPriority,
    pub code_location: ReviewCodeLocation,
}

impl ReviewFinding {
    /// The title with any leading `[P0]`..`[P3]` tag removed.
    ///
    /// The rubric asks the model to prefix each title with its priority, and
    /// every surface renders the priority itself, so surfaces must strip the
    /// model's tag or it shows up twice (`[P3] [P3] …`). Tolerates lowercase
    /// and missing tags alike.
    pub fn display_title(&self) -> &str {
        let title = self.title.trim();
        let Some(rest) = title.strip_prefix('[') else {
            return title;
        };
        let Some((tag, rest)) = rest.split_once(']') else {
            return title;
        };
        let tag = tag.trim();
        let is_priority_tag = tag.len() == 2
            && tag.starts_with(['p', 'P'])
            && tag[1..].chars().all(|digit| ('0'..='3').contains(&digit));
        if is_priority_tag {
            rest.trim_start()
        } else {
            title
        }
    }
}

/// Structured result of a review turn. Every field is optional so a partially
/// parsed model response still round-trips.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOutput {
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_correctness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_explanation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_confidence_score: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPublisherDescriptor {
    pub id: ReviewPublisherId,
    pub display_name: String,
    pub capabilities: ReviewPublisherCapabilities,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPublisherCapabilities {
    /// Can attach comments to specific file/line locations.
    pub inline_comments: bool,
    /// Can post an overall summary alongside the comments.
    pub summary_body: bool,
    /// Can infer the destination (PR number, repo) from the workspace alone.
    pub autodetect_destination: bool,
    /// Can render the payload without performing any remote call.
    pub dry_run: bool,
}

/// Free-form publish destination; each publisher interprets `target` and
/// `options` in its own terms.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDestination {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_id: Option<ReviewPublisherId>,
    /// e.g. `owner/repo#123`, a Linear issue id, a chat channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default)]
    pub options: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ReviewPublishRequest {
    pub review_id: ReviewId,
    pub workspace_root: PathBuf,
    /// Carries `base_sha` / `head_sha` for anchoring comments.
    pub request: ReviewRequest,
    pub output: ReviewOutput,
    /// Indices into `output.findings`; `None` publishes all of them.
    pub selected: Option<Vec<usize>>,
    pub destination: ReviewDestination,
    pub dry_run: bool,
}

/// A finding the publisher could not place (e.g. outside the reviewed diff).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPublishSkip {
    pub finding_index: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPublishResult {
    pub publisher_id: ReviewPublisherId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub published_findings: usize,
    #[serde(default)]
    pub skipped: Vec<ReviewPublishSkip>,
    /// Populated instead of a remote call when `dry_run` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_preview: Option<serde_json::Value>,
}

#[derive(Debug)]
pub enum ReviewPublishError {
    /// The publisher could not work out where to publish.
    DestinationUnresolved(String),
    /// The publisher exists but has no usable credentials/binary.
    NotConfigured(String),
    /// The remote refused the review.
    Remote(String),
    Other(anyhow::Error),
}

impl std::fmt::Display for ReviewPublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DestinationUnresolved(message) => {
                write!(f, "destination could not be resolved: {message}")
            }
            Self::NotConfigured(message) => write!(f, "publisher not configured: {message}"),
            Self::Remote(message) => write!(f, "remote rejected the review: {message}"),
            Self::Other(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ReviewPublishError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Other(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for ReviewPublishError {
    fn from(error: anyhow::Error) -> Self {
        Self::Other(error)
    }
}

/// Submits review findings to an external platform. Implemented by
/// `roder-ext-*` crates and registered on the extension registry; nothing in
/// core knows about any concrete platform.
#[async_trait::async_trait]
pub trait ReviewPublisher: Send + Sync + 'static {
    fn descriptor(&self) -> ReviewPublisherDescriptor;

    /// Cheap availability probe (binary on `PATH`, token present). Advisory:
    /// a `false` answer should not be treated as a hard failure.
    async fn is_available(&self, workspace_root: &Path) -> bool {
        let _ = workspace_root;
        true
    }

    async fn publish(
        &self,
        request: ReviewPublishRequest,
    ) -> Result<ReviewPublishResult, ReviewPublishError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_title_strips_only_a_leading_priority_tag() {
        let finding = |title: &str| ReviewFinding {
            title: title.to_string(),
            body: String::new(),
            confidence_score: None,
            priority: ReviewPriority::P1,
            code_location: ReviewCodeLocation {
                absolute_file_path: PathBuf::from("/tmp/a.rs"),
                line_range: ReviewLineRange { start: 1, end: 1 },
            },
        };

        assert_eq!(
            finding("[P1] Guard the divisor").display_title(),
            "Guard the divisor"
        );
        assert_eq!(finding("[p0]  Guard").display_title(), "Guard");
        assert_eq!(finding("  [P3] Guard  ").display_title(), "Guard");
        // No tag, an unrelated bracket, and an out-of-range tag all survive.
        assert_eq!(
            finding("Guard the divisor").display_title(),
            "Guard the divisor"
        );
        assert_eq!(finding("[BUG] Guard").display_title(), "[BUG] Guard");
        assert_eq!(finding("[P9] Guard").display_title(), "[P9] Guard");
        assert_eq!(finding("[P1 Guard").display_title(), "[P1 Guard");
    }

    #[test]
    fn review_target_is_tagged_and_camel_case() {
        let target = ReviewTarget::BaseBranch {
            branch: "main".to_string(),
        };
        let value = serde_json::to_value(&target).expect("serialize target");
        assert_eq!(value["kind"], "baseBranch");
        assert_eq!(value["branch"], "main");

        let round_trip: ReviewTarget = serde_json::from_value(value).expect("deserialize target");
        assert_eq!(round_trip, target);
        assert_eq!(ReviewTarget::default(), ReviewTarget::UncommittedChanges);
    }

    #[test]
    fn review_priority_defaults_to_p2_and_parses() {
        assert_eq!(ReviewPriority::default(), ReviewPriority::P2);
        assert_eq!(ReviewPriority::parse("P1"), Some(ReviewPriority::P1));
        assert_eq!(ReviewPriority::parse(" p3 "), Some(ReviewPriority::P3));
        assert_eq!(ReviewPriority::parse("urgent"), None);
        assert_eq!(ReviewPriority::P0.to_string(), "p0");
        assert!(ReviewPriority::P0 < ReviewPriority::P3);
        assert_eq!(
            serde_json::to_value(ReviewPriority::P1).expect("serialize priority"),
            serde_json::json!("p1")
        );
    }

    #[test]
    fn review_finding_defaults_missing_priority() {
        let finding: ReviewFinding = serde_json::from_value(serde_json::json!({
            "title": "Off-by-one",
            "body": "The loop overruns.",
            "codeLocation": {
                "absoluteFilePath": "/repo/src/lib.rs",
                "lineRange": { "start": 10, "end": 12 }
            }
        }))
        .expect("deserialize finding");

        assert_eq!(finding.priority, ReviewPriority::P2);
        assert!(finding.confidence_score.is_none());
        assert_eq!(finding.code_location.line_range.end, 12);
    }

    #[test]
    fn review_output_tolerates_an_empty_object() {
        let output: ReviewOutput =
            serde_json::from_str("{}").expect("deserialize empty review output");
        assert!(output.findings.is_empty());
        assert!(output.overall_correctness.is_none());
    }

    #[test]
    fn publish_error_renders_context() {
        let error = ReviewPublishError::NotConfigured("gh not on PATH".to_string());
        assert_eq!(
            error.to_string(),
            "publisher not configured: gh not on PATH"
        );
    }
}
