use roder_api::plan_review::{
    HunkDiffLine, HunkDiffLineKind, HunkRecord, HunkRollbackState, MAX_HUNK_DIFF_LINES,
};
use roder_api::tools::{ToolCall, ToolExecutionContext};
use time::OffsetDateTime;

pub(crate) fn hunk_id(call: &ToolCall, index: usize) -> String {
    format!("{}-hunk-{}", call.id, index + 1)
}

pub(crate) fn record(
    ctx: &ToolExecutionContext,
    call: &ToolCall,
    index: usize,
    path: impl Into<String>,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
) -> HunkRecord {
    let path = path.into();
    let mut diff = Vec::new();
    for (old_line, line) in (1u32..).zip(old_lines.iter()) {
        diff.push(HunkDiffLine {
            kind: HunkDiffLineKind::Removed,
            text: line.clone(),
            old_line: Some(old_line),
            new_line: None,
        });
    }
    for (new_line, line) in (1u32..).zip(new_lines.iter()) {
        diff.push(HunkDiffLine {
            kind: HunkDiffLineKind::Added,
            text: line.clone(),
            old_line: None,
            new_line: Some(new_line),
        });
    }
    if diff.len() > MAX_HUNK_DIFF_LINES {
        diff.truncate(MAX_HUNK_DIFF_LINES);
    }

    HunkRecord {
        id: hunk_id(call, index),
        thread_id: ctx.thread_id.clone(),
        turn_id: ctx.turn_id.clone(),
        path: path.clone(),
        old_start: 1,
        old_lines: old_lines.len() as u32,
        new_start: 1,
        new_lines: new_lines.len() as u32,
        diff,
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        plan_review_id: None,
        plan_step_id: None,
        timeline_event_id: None,
        checkpoint_id: None,
        rollback: HunkRollbackState::Available,
        reverse_patch: Some(reverse_codex_patch(&path, &old_lines, &new_lines)),
        created_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn reverse_codex_patch(path: &str, old_lines: &[String], new_lines: &[String]) -> String {
    let mut patch = String::from("*** Begin Patch\n");
    patch.push_str(&format!("*** Update File: {path}\n@@\n"));
    for line in new_lines {
        patch.push('-');
        patch.push_str(line);
        patch.push('\n');
    }
    for line in old_lines {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    patch.push_str("*** End Patch\n");
    patch
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::policy_mode::PolicyMode;
    use serde_json::json;

    #[test]
    fn hunk_record_preserves_tool_and_turn_ids() {
        let ctx = ToolExecutionContext::new("thread-1", "turn-1", PolicyMode::Default);
        let call = ToolCall {
            id: "tool-1".to_string(),
            name: "edit".to_string(),
            arguments: json!({}),
            raw_arguments: "{}".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        };
        let hunk = record(
            &ctx,
            &call,
            0,
            "src/lib.rs",
            vec!["old".to_string()],
            vec!["new".to_string()],
        );

        assert_eq!(hunk.id, "tool-1-hunk-1");
        assert_eq!(hunk.thread_id, "thread-1");
        assert_eq!(hunk.tool_name, "edit");
        assert_eq!(hunk.diff.len(), 2);
    }
}
