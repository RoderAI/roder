use ratatui::layout::Rect;
use roder_api::plan_review::{
    HunkDiffLine, HunkDiffLineKind, HunkRecord, HunkRollbackState, PlanReview, PlanReviewStatus,
    PlanStep,
};
use time::OffsetDateTime;

use super::Theme;
use super::tool_timeline::TimelineState;

fn rendered_lines(timeline: &mut TimelineState) -> Vec<String> {
    timeline
        .render(Theme::for_dark_background(true), Rect::new(0, 0, 100, 16))
        .text
        .lines
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect()
}

fn review() -> PlanReview {
    let now = OffsetDateTime::UNIX_EPOCH;
    PlanReview {
        id: "review-1".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        status: PlanReviewStatus::AwaitingReview,
        title: "Implement patch".to_string(),
        markdown: "- Edit src/lib.rs\n- Run tests".to_string(),
        steps: vec![PlanStep {
            id: "step-1".to_string(),
            title: "Edit src/lib.rs".to_string(),
            detail: None,
            completed: false,
        }],
        comments: Vec::new(),
        rewrites: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

fn hunk() -> HunkRecord {
    HunkRecord {
        id: "hunk-1".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        path: "src/lib.rs".to_string(),
        old_start: 1,
        old_lines: 1,
        new_start: 1,
        new_lines: 1,
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
        reverse_patch: Some("*** Begin Patch\n".to_string()),
        created_at: OffsetDateTime::UNIX_EPOCH,
    }
}

#[test]
fn plan_review_renders_without_standalone_hunk_rows() {
    let mut timeline = TimelineState::default();
    timeline.record_plan_review_created(review());
    timeline.record_hunk(hunk());

    let lines = rendered_lines(&mut timeline);
    assert!(lines.iter().any(|line| line.contains("plan review")));
    assert!(!lines.iter().any(|line| line.contains("hunk hunk-1")));
    assert!(!lines.iter().any(|line| line.contains("+new")));
}

#[test]
fn standalone_hunk_records_are_not_rendered() {
    let mut timeline = TimelineState::default();
    let mut standalone = hunk();
    standalone.plan_review_id = None;

    timeline.record_hunk(standalone);

    let lines = rendered_lines(&mut timeline);
    assert!(!lines.iter().any(|line| line.contains("hunk hunk-1")));
    assert!(!lines.iter().any(|line| line.contains("src/lib.rs")));
    assert!(!lines.iter().any(|line| line.contains("+new")));
}
