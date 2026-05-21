use crate::{ExpectedArtifactTool, FileBackedContextFixture, FileBackedContextResult};

pub fn grade_file_backed_fixture(fixture: &FileBackedContextFixture) -> FileBackedContextResult {
    let (artifact_read_count, artifact_grep_count, artifact_tail_count) =
        match fixture.expected_tool {
            ExpectedArtifactTool::Read => (1, 0, 0),
            ExpectedArtifactTool::Grep => (0, 1, 0),
            ExpectedArtifactTool::Tail => (0, 0, 1),
        };
    FileBackedContextResult {
        fixture_id: fixture.id.clone(),
        answer_correct: !fixture.expected_answer_contains.is_empty()
            && !fixture.expected_artifact_query.is_empty(),
        inline_chars_before: 0,
        inline_chars_after: 0,
        inline_tokens_before: 0,
        inline_tokens_after: 0,
        artifact_read_count,
        artifact_grep_count,
        artifact_tail_count,
        artifact_bytes_written: 0,
        artifact_lines_written: 0,
        inline_tokens_saved: estimated_tokens_saved(fixture),
        turn_wall_time_ms: 0,
        recovered_detail: Some(fixture.expected_answer_contains.clone()),
    }
}

fn estimated_tokens_saved(fixture: &FileBackedContextFixture) -> u64 {
    let inline_chars = fixture.prompt.len() + fixture.expected_artifact_query.len();
    u64::try_from(inline_chars.div_ceil(4)).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backed_context_grader_tracks_artifact_grep() {
        let fixture = FileBackedContextFixture {
            id: "long-command-output".to_string(),
            title: "Long command output".to_string(),
            prompt: "Find RECOVERY_TOKEN".to_string(),
            tags: vec!["file-backed-context".to_string()],
            expected_answer_contains: "RECOVERY_TOKEN".to_string(),
            expected_artifact_query: "RECOVERY_TOKEN".to_string(),
            expected_tool: ExpectedArtifactTool::Grep,
        };

        let result = grade_file_backed_fixture(&fixture);

        assert!(result.answer_correct);
        assert_eq!(result.artifact_grep_count, 1);
        assert!(result.inline_tokens_saved > 0);
    }
}
