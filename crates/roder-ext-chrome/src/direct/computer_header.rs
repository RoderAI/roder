//! The line that heads a native computer batch's result when the batch did not
//! simply succeed: what failed, counted against what was asked.

/// Why a batch did not go through.
pub(super) enum Failure {
    /// The batch was turned away before any action ran.
    Refused(String),
    /// The action after the completed ones failed.
    Action(String),
}

impl Failure {
    pub(super) fn message(&self) -> &str {
        match self {
            Self::Refused(message) | Self::Action(message) => message,
        }
    }
}

/// The header for a batch of `requested` actions of which `completed` ran to
/// the end, empty when nothing failed. `cleanup` is the error of releasing
/// input after the batch, when releasing failed: it is said beside a failed
/// action, and on its own when every action that ran had completed.
pub(super) fn header(
    failure: Option<&Failure>,
    cleanup: Option<&str>,
    completed: usize,
    requested: usize,
) -> String {
    let mut lines = Vec::new();
    match (failure, cleanup) {
        (None, None) => return String::new(),
        (Some(Failure::Action(message)), _) => lines.push(format!(
            "Computer action {} of {requested} failed: {message}",
            completed + 1
        )),
        (Some(Failure::Refused(message)), _) => lines.push(format!(
            "No computer action was run: {message} (this batch has {requested} actions)."
        )),
        (None, Some(cleanup)) => lines.push(format!(
            "{}, but cleanup afterwards failed: {cleanup}",
            completed_phrase(completed, requested)
        )),
    }
    if let (Some(_), Some(cleanup)) = (failure, cleanup) {
        lines.push(format!("Cleanup afterwards failed too: {cleanup}"));
    }
    lines.push(String::new());
    lines.join("\n")
}

/// How many of the actions ran to the end, said as a clause.
fn completed_phrase(completed: usize, requested: usize) -> String {
    match (completed == requested, requested) {
        (true, 1) => "The computer action completed".to_string(),
        (true, _) => format!("All {requested} computer actions completed"),
        (false, _) => format!("{completed} of {requested} computer actions completed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(message: &str) -> Failure {
        Failure::Action(message.to_string())
    }

    #[test]
    fn a_failed_action_is_counted_from_one_against_the_batch() {
        assert_eq!(
            header(Some(&action("boom")), None, 1, 3),
            "Computer action 2 of 3 failed: boom\n"
        );
        assert_eq!(
            header(Some(&action("boom")), None, 0, 1),
            "Computer action 1 of 1 failed: boom\n"
        );
    }

    #[test]
    fn nothing_failed_is_no_header() {
        assert_eq!(header(None, None, 3, 3), "");
    }

    #[test]
    fn when_every_action_completed_and_only_the_cleanup_failed_the_header_says_so() {
        let text = header(None, Some("input remains blocked"), 3, 3);
        assert!(
            text.starts_with("All 3 computer actions completed"),
            "{text}"
        );
        assert!(text.contains("cleanup afterwards failed"), "{text}");
        assert!(text.contains("input remains blocked"), "{text}");
        assert!(
            !text.contains("4 of 3") && !text.contains("action 4"),
            "no action past the last one is blamed: {text}"
        );
        let single = header(None, Some("x"), 1, 1);
        assert!(
            single.starts_with("The computer action completed"),
            "{single}"
        );
    }

    #[test]
    fn a_batch_that_stopped_early_and_then_failed_to_clean_up_counts_what_ran() {
        let text = header(None, Some("input remains blocked"), 1, 4);
        assert!(
            text.starts_with("1 of 4 computer actions completed"),
            "{text}"
        );
        assert!(text.contains("cleanup afterwards failed"), "{text}");
    }

    #[test]
    fn a_failed_action_is_not_hidden_by_a_failed_cleanup() {
        let text = header(Some(&action("boom")), Some("input remains blocked"), 1, 3);
        assert!(
            text.starts_with("Computer action 2 of 3 failed: boom\n"),
            "{text}"
        );
        assert!(text.contains("Cleanup afterwards failed too"), "{text}");
        assert!(text.contains("input remains blocked"), "{text}");
    }

    #[test]
    fn a_refused_batch_is_not_an_action_that_failed() {
        for requested in [0, 101] {
            let refused = Failure::Refused("computer requires 1..100 actions".into());
            let text = header(Some(&refused), None, 0, requested);
            assert!(
                text.starts_with("No computer action was run: computer requires 1..100 actions"),
                "{text}"
            );
            assert!(
                text.contains(&format!("{requested} actions")),
                "it says how many were given: {text}"
            );
            assert!(!text.contains("action 1 of"), "{text}");
        }
    }
}
