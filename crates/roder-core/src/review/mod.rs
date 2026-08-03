//! Read-only code review sub-turns.
//!
//! A review runs the rubric below as the system prompt of a single turn on a
//! detached, read-only child thread, then parses the model's final message into
//! the structured [`roder_api::review::ReviewOutput`] the TUI renders and
//! review publishers submit.

mod parse;
mod prompt;
mod publish;
mod render;
mod run;
#[cfg(test)]
mod test_vcs;

/// System prompt for the review turn. Its output schema is the serde shape of
/// [`roder_api::review::ReviewOutput`]; changing one without the other silently
/// drops every finding into the parser's raw-text fallback.
pub const REVIEW_RUBRIC: &str = include_str!("rubric.md");

pub use parse::parse_review_output;
pub use prompt::{resolve_review_request, review_label};
pub use publish::{PublishReviewError, PublishReviewRequest, RuntimeReviewConfig};
pub use render::{
    REVIEW_FALLBACK_MESSAGE, render_findings, render_review_output_text, synthetic_user_action,
    synthetic_user_action_interrupted,
};
pub use run::{
    REVIEW_TOOL_ALLOWLIST, ReviewCompletion, ReviewHandle, ReviewRecord, StartReviewRequest,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rubric_documents_the_serde_shape_of_the_output() {
        for key in [
            "\"findings\"",
            "\"title\"",
            "\"body\"",
            "\"confidenceScore\"",
            "\"priority\"",
            "\"codeLocation\"",
            "\"absoluteFilePath\"",
            "\"lineRange\"",
            "\"overallCorrectness\"",
            "\"overallExplanation\"",
            "\"overallConfidenceScore\"",
        ] {
            assert!(
                REVIEW_RUBRIC.contains(key),
                "rubric must document the {key} field"
            );
        }
        assert!(REVIEW_RUBRIC.contains("\"start\""));
        assert!(REVIEW_RUBRIC.contains("\"end\""));
    }

    /// The rubric's JSON example must actually deserialize into the type the
    /// parser produces, so a schema drift fails here rather than at runtime.
    #[test]
    fn rubric_example_round_trips_through_the_parser() {
        let example = REVIEW_RUBRIC
            .split("```json")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .expect("rubric must contain a fenced json example");
        let example = example.replace(
            "<= 80 chars, imperative, prefixed with the priority tag>",
            "[P1] Off-by-one",
        );
        let output = parse_review_output(&example);
        assert_eq!(output.findings.len(), 1);
        assert_eq!(
            output.findings[0].priority,
            roder_api::review::ReviewPriority::P1
        );
        assert_eq!(
            output.findings[0].code_location.line_range.start, 120,
            "rubric lineRange must decode"
        );
        assert_eq!(
            output.overall_correctness.as_deref(),
            Some("patch is correct")
        );
        assert_eq!(output.overall_confidence_score, Some(0.82));
    }

    #[test]
    fn review_tool_allowlist_is_read_only() {
        for tool in [
            "write_file",
            "edit",
            "multi_edit",
            "apply_patch",
            "exec_command",
            "unified_exec",
            "write_stdin",
            "task",
        ] {
            assert!(
                !REVIEW_TOOL_ALLOWLIST.contains(&tool),
                "{tool} must not be reachable from a review turn"
            );
        }
        assert!(REVIEW_TOOL_ALLOWLIST.contains(&"shell"));
        assert!(REVIEW_TOOL_ALLOWLIST.contains(&"read_file"));
    }
}
