//! Turns a [`ReviewOutput`] into the two plain-text messages that get echoed
//! into the thread the review was requested from: a synthetic user action that
//! tells the parent agent a review happened, and the assistant-facing summary.

use roder_api::review::{ReviewFinding, ReviewOutput};

/// Shown when a review produced neither findings nor an explanation.
pub const REVIEW_FALLBACK_MESSAGE: &str = "Reviewer failed to output a response.";

fn format_location(finding: &ReviewFinding) -> String {
    let path = finding.code_location.absolute_file_path.display();
    let start = finding.code_location.line_range.start;
    let end = finding.code_location.line_range.end;
    format!("{path}:{start}-{end}")
}

/// Renders the findings as a bullet list — `- [P1] Title — /abs/path:12-14`
/// with the body indented two spaces underneath. Empty when there are none.
pub fn render_findings(output: &ReviewOutput) -> String {
    if output.findings.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    lines.push(if output.findings.len() > 1 {
        "Full review comments:".to_string()
    } else {
        "Review comment:".to_string()
    });

    for finding in &output.findings {
        lines.push(String::new());
        lines.push(format!(
            "- [{}] {} — {}",
            finding.priority.to_string().to_uppercase(),
            finding.display_title(),
            format_location(finding)
        ));
        for body_line in finding.body.lines() {
            lines.push(format!("  {body_line}"));
        }
    }

    lines.join("\n")
}

/// Human-readable summary of a review: the overall explanation, the findings
/// block, or both separated by a blank line.
pub fn render_review_output_text(output: &ReviewOutput) -> String {
    let mut sections = Vec::new();
    if let Some(explanation) = output.overall_explanation.as_deref() {
        let explanation = explanation.trim();
        if !explanation.is_empty() {
            sections.push(explanation.to_string());
        }
    }
    let findings = render_findings(output);
    if !findings.is_empty() {
        sections.push(findings);
    }
    if sections.is_empty() {
        REVIEW_FALLBACK_MESSAGE.to_string()
    } else {
        sections.join("\n\n")
    }
}

/// The synthetic user turn recorded in the requesting thread so the parent
/// agent knows a review ran and can act on the findings the user keeps.
pub fn synthetic_user_action(output: &ReviewOutput) -> String {
    let results = indent_results(&render_review_output_text(output));
    format!(
        "<user_action>\n  <context>User initiated a review task. Here's the full review output from reviewer model. User may select one or more comments to resolve.</context>\n  <action>review</action>\n  <results>\n{results}\n  </results>\n</user_action>"
    )
}

/// Counterpart of [`synthetic_user_action`] for a review that never produced
/// output, so the parent thread does not silently gain an unexplained turn.
pub fn synthetic_user_action_interrupted() -> String {
    "<user_action>\n  <context>User initiated a review task, but it was interrupted. If the user asks about this, tell them to re-run `/review` and wait for it to complete.</context>\n  <action>review</action>\n  <results>\n  None.\n  </results>\n</user_action>".to_string()
}

fn indent_results(results: &str) -> String {
    results
        .lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("  {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use roder_api::review::{ReviewCodeLocation, ReviewLineRange, ReviewPriority};

    use super::*;

    fn finding(title: &str, priority: ReviewPriority, body: &str) -> ReviewFinding {
        ReviewFinding {
            title: title.to_string(),
            body: body.to_string(),
            confidence_score: Some(0.5),
            priority,
            code_location: ReviewCodeLocation {
                absolute_file_path: PathBuf::from("/repo/src/lib.rs"),
                line_range: ReviewLineRange { start: 10, end: 12 },
            },
        }
    }

    #[test]
    fn renders_a_single_finding_with_an_indented_body() {
        let output = ReviewOutput {
            findings: vec![finding(
                "Off-by-one",
                ReviewPriority::P1,
                "Overruns.\nTwice.",
            )],
            ..ReviewOutput::default()
        };

        assert_eq!(
            render_findings(&output),
            "Review comment:\n\n- [P1] Off-by-one — /repo/src/lib.rs:10-12\n  Overruns.\n  Twice."
        );
    }

    #[test]
    fn renders_multiple_findings_with_a_plural_header() {
        let output = ReviewOutput {
            findings: vec![
                finding("First", ReviewPriority::P0, "a"),
                finding("Second", ReviewPriority::P3, "b"),
            ],
            ..ReviewOutput::default()
        };

        let rendered = render_findings(&output);
        assert!(rendered.starts_with("Full review comments:"));
        assert!(rendered.contains("- [P0] First — /repo/src/lib.rs:10-12"));
        assert!(rendered.contains("- [P3] Second — /repo/src/lib.rs:10-12"));
    }

    #[test]
    fn renders_nothing_for_an_empty_output() {
        assert_eq!(render_findings(&ReviewOutput::default()), "");
        assert_eq!(
            render_review_output_text(&ReviewOutput::default()),
            REVIEW_FALLBACK_MESSAGE
        );
    }

    #[test]
    fn summary_joins_the_explanation_and_the_findings() {
        let output = ReviewOutput {
            findings: vec![finding("Off-by-one", ReviewPriority::P1, "Overruns.")],
            overall_explanation: Some("  One blocking bug.  ".to_string()),
            ..ReviewOutput::default()
        };

        assert_eq!(
            render_review_output_text(&output),
            "One blocking bug.\n\nReview comment:\n\n- [P1] Off-by-one — /repo/src/lib.rs:10-12\n  Overruns."
        );
    }

    #[test]
    fn synthetic_user_action_wraps_and_indents_the_results() {
        let output = ReviewOutput {
            overall_explanation: Some("Looks fine.".to_string()),
            ..ReviewOutput::default()
        };

        let rendered = synthetic_user_action(&output);
        assert!(rendered.starts_with("<user_action>\n"));
        assert!(rendered.contains("<action>review</action>"));
        assert!(rendered.contains("\n  <results>\n  Looks fine.\n  </results>\n"));
        assert!(rendered.ends_with("</user_action>"));
    }

    #[test]
    fn interrupted_user_action_reports_no_results() {
        let rendered = synthetic_user_action_interrupted();
        assert!(rendered.contains("interrupted"));
        assert!(rendered.contains("None."));
    }
}
