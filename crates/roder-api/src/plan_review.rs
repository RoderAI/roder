use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};

pub type PlanReviewId = String;
pub type PlanStepId = String;
pub type PlanCommentId = String;
pub type PlanRewriteId = String;
pub type HunkId = String;

pub const MAX_PLAN_REVIEW_TEXT_CHARS: usize = 64 * 1024;
pub const MAX_HUNK_DIFF_LINES: usize = 400;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlanReviewStatus {
    Drafted,
    AwaitingReview,
    Rewritten,
    Approved,
    Executing,
    Completed,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlanCommentAnchor {
    WholePlan,
    #[serde(rename_all = "camelCase")]
    Step {
        step_id: PlanStepId,
    },
    #[serde(rename_all = "camelCase")]
    File {
        path: String,
        start_line: Option<u32>,
        end_line: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Hunk {
        hunk_id: HunkId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub id: PlanStepId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default)]
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanComment {
    pub id: PlanCommentId,
    pub review_id: PlanReviewId,
    pub anchor: PlanCommentAnchor,
    pub body: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanRewrite {
    pub id: PlanRewriteId,
    pub review_id: PlanReviewId,
    pub replacement_markdown: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanReview {
    pub id: PlanReviewId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub status: PlanReviewStatus,
    pub title: String,
    pub markdown: String,
    #[serde(default)]
    pub steps: Vec<PlanStep>,
    #[serde(default)]
    pub comments: Vec<PlanComment>,
    #[serde(default)]
    pub rewrites: Vec<PlanRewrite>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HunkRollbackState {
    Unavailable { reason: String },
    Available,
    Applied,
    Conflict { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HunkDiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HunkDiffLine {
    pub kind: HunkDiffLineKind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HunkRecord {
    pub id: HunkId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub path: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    #[serde(default)]
    pub diff: Vec<HunkDiffLine>,
    pub tool_call_id: String,
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_review_id: Option<PlanReviewId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_step_id: Option<PlanStepId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    pub rollback: HunkRollbackState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_patch: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PagedHunkDiff {
    pub hunk: HunkRecord,
    pub offset: usize,
    pub limit: usize,
    pub total_lines: usize,
    pub lines: Vec<HunkDiffLine>,
    pub next_offset: Option<usize>,
}

pub fn cap_text(mut text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }
    text = text.chars().take(max_chars).collect();
    text.push_str("\n[truncated]");
    text
}

pub fn page_hunk_diff(hunk: HunkRecord, offset: usize, limit: usize) -> PagedHunkDiff {
    let total_lines = hunk.diff.len();
    let start = offset.min(total_lines);
    let end = start.saturating_add(limit).min(total_lines);
    let lines = hunk.diff[start..end].to_vec();
    PagedHunkDiff {
        hunk,
        offset: start,
        limit,
        total_lines,
        lines,
        next_offset: (end < total_lines).then_some(end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_review_uses_camel_case_and_stable_ids() {
        let review = PlanReview {
            id: "review-1".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            status: PlanReviewStatus::AwaitingReview,
            title: "Plan".to_string(),
            markdown: "- step".to_string(),
            steps: vec![PlanStep {
                id: "step-1".to_string(),
                title: "Edit file".to_string(),
                detail: None,
                completed: false,
            }],
            comments: vec![PlanComment {
                id: "comment-1".to_string(),
                review_id: "review-1".to_string(),
                anchor: PlanCommentAnchor::Step {
                    step_id: "step-1".to_string(),
                },
                body: "Tighten this.".to_string(),
                created_at: OffsetDateTime::UNIX_EPOCH,
            }],
            rewrites: vec![],
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        };

        let value = serde_json::to_value(&review).unwrap();
        assert_eq!(value["threadId"], "thread-1");
        assert_eq!(value["status"], "awaitingReview");
        assert_eq!(value["steps"][0]["id"], "step-1");

        let round_trip: PlanReview = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip.id, "review-1");
    }

    #[test]
    fn hunk_diff_pages_are_bounded() {
        let hunk = HunkRecord {
            id: "hunk-1".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            path: "src/lib.rs".to_string(),
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 2,
            diff: vec![
                HunkDiffLine {
                    kind: HunkDiffLineKind::Removed,
                    text: "old".to_string(),
                    old_line: Some(1),
                    new_line: None,
                },
                HunkDiffLine {
                    kind: HunkDiffLineKind::Added,
                    text: "new".to_string(),
                    old_line: None,
                    new_line: Some(1),
                },
            ],
            tool_call_id: "tool-1".to_string(),
            tool_name: "apply_patch".to_string(),
            plan_review_id: Some("review-1".to_string()),
            plan_step_id: Some("step-1".to_string()),
            timeline_event_id: None,
            checkpoint_id: None,
            rollback: HunkRollbackState::Available,
            reverse_patch: Some("*** Begin Patch".to_string()),
            created_at: OffsetDateTime::UNIX_EPOCH,
        };

        let page = page_hunk_diff(hunk, 0, 1);
        assert_eq!(page.lines.len(), 1);
        assert_eq!(page.next_offset, Some(1));
    }
}
