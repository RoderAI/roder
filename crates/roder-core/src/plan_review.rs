use roder_api::events::{PlanReviewCreated, RoderEvent, ThreadId, TurnId};
use roder_api::plan_review::{PlanReview, PlanReviewStatus, PlanStep};
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::ToolCall;
use time::OffsetDateTime;

pub(crate) fn plan_review_for_blocked_tool(
    thread_id: &ThreadId,
    turn_id: &TurnId,
    call: &ToolCall,
    mode: PolicyMode,
) -> Option<RoderEvent> {
    if mode != PolicyMode::Plan || !is_file_changing_tool(&call.name) {
        return None;
    }
    let now = OffsetDateTime::now_utc();
    let target = call
        .arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            call.arguments
                .get("patch")
                .and_then(serde_json::Value::as_str)
                .and_then(first_patch_path)
        })
        .unwrap_or("workspace");
    let title = format!("Review {} before editing {}", call.name, target);
    let review = PlanReview {
        id: format!("{}-plan-review", call.id),
        thread_id: thread_id.clone(),
        turn_id: turn_id.clone(),
        status: PlanReviewStatus::AwaitingReview,
        title: title.clone(),
        markdown: format!("- {title}"),
        steps: vec![PlanStep {
            id: format!("{}-step-1", call.id),
            title,
            detail: Some("Plan mode blocks file-changing tools until the user leaves plan mode or explicitly bypasses policy.".to_string()),
            completed: false,
        }],
        comments: Vec::new(),
        rewrites: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    Some(RoderEvent::PlanReviewCreated(PlanReviewCreated {
        review,
        timestamp: now,
    }))
}

fn is_file_changing_tool(name: &str) -> bool {
    matches!(
        name,
        "apply_patch" | "write_file" | "edit" | "multi_edit" | "fs.write" | "fs.edit"
    )
}

fn first_patch_path(patch: &str) -> Option<&str> {
    patch.lines().find_map(|line| {
        line.strip_prefix("*** Update File: ")
            .or_else(|| line.strip_prefix("*** Add File: "))
            .or_else(|| line.strip_prefix("*** Delete File: "))
            .map(str::trim)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_review_event_is_created_for_blocked_patch_tool() {
        let call = ToolCall {
            id: "tool-1".to_string(),
            name: "apply_patch".to_string(),
            arguments: json!({
                "patch": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch\n"
            }),
            raw_arguments: "{}".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        };
        let event = plan_review_for_blocked_tool(
            &"thread-1".to_string(),
            &"turn-1".to_string(),
            &call,
            PolicyMode::Plan,
        )
        .unwrap();

        let RoderEvent::PlanReviewCreated(created) = event else {
            panic!("expected plan review event");
        };
        assert_eq!(created.review.id, "tool-1-plan-review");
        assert_eq!(created.review.steps[0].id, "tool-1-step-1");
        assert!(created.review.markdown.contains("src/lib.rs"));
    }
}
