//! When a run falls back: Jev could not progress, and a different driver
//! might. It ended `blocked` because the model answered BLOCKED, three steps
//! changed nothing, its targets stayed covered, or it answered DONE after a
//! covered attempt; the page offered nothing Jev can act on; or its budget
//! ran out short of the goal.
//!
//! Never when a different driver does not fix the cause, and never as a way
//! around a stop that protects the user: `needs_input` (a value the goal
//! lacks), `needs_confirmation` (the irreversible-action gate),
//! `access_denied` (a bot wall, a CAPTCHA, a refusal), a page that did not
//! load, a page outside the allowed origins, a confirm or prompt Jev
//! declined, a provider that could not be reached, an error, or a timeout
//! that has spent the call's time.

use serde::Serialize;
use serde_json::json;

use crate::engine::{JevRunResult, JevStatus, JevStopCause};

/// Why a run falls back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Trigger {
    /// The model answered BLOCKED on a page that offered controls.
    ModelBlocked,
    /// A model DONE was rejected by independent UI postconditions.
    OutcomeMismatch,
    /// Three actions in a row changed nothing.
    Stalled,
    /// Its targets stayed covered (three covered attempts, or DONE after
    /// one).
    Covered,
    /// The page offered nothing Jev can act on.
    NothingToActOn,
    /// The action or model-call budget ran out.
    Budget,
}

impl Trigger {
    /// Said in the result and to the fallback model.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::ModelBlocked => {
                "Jev's model judged it could not go on with the controls it can use"
            }
            Self::OutcomeMismatch => {
                "Jev reported DONE, but fresh UI state failed the caller-defined success condition"
            }
            Self::Stalled => "three of Jev's actions in a row changed nothing on the page",
            Self::Covered => "the controls Jev tried were covered by another element",
            Self::NothingToActOn => {
                "the page offered nothing Jev can act on (it cannot use canvas drawings, drags, \
                 hover-only menus or coordinates)"
            }
            Self::Budget => "Jev ran out of its action budget short of the goal",
        }
    }
}

/// Whether, and why, `result` falls back.
pub(crate) fn trigger(result: &JevRunResult) -> Option<Trigger> {
    match result.status {
        JevStatus::Blocked => {
            if declined_a_dialog(result) {
                return None;
            }
            let cause = match result.stop_cause? {
                JevStopCause::OutcomeMismatch => Trigger::OutcomeMismatch,
                JevStopCause::ModelBlocked => Trigger::ModelBlocked,
                JevStopCause::Stalled => Trigger::Stalled,
                JevStopCause::Covered | JevStopCause::DoneAfterCovered => Trigger::Covered,
                _ => return None,
            };
            Some(match result.observed_elements {
                0 => Trigger::NothingToActOn,
                _ => cause,
            })
        }
        JevStatus::BudgetExceeded if result.stop_cause == Some(JevStopCause::Budget) => {
            Some(Trigger::Budget)
        }
        _ => None,
    }
}

/// The page asked for consent or input and Jev declined: whoever goes on
/// would face the same question, which is the caller's.
fn declined_a_dialog(result: &JevRunResult) -> bool {
    result
        .actions
        .iter()
        .flat_map(|action| &action.dialogs)
        .any(|dialog| !dialog.accepted)
        || result
            .stopped_because
            .as_deref()
            .is_some_and(|reason| reason.contains("Jev never accepts a confirm"))
}

/// The trigger as the result data records it.
pub(crate) fn recorded(trigger: Trigger) -> serde_json::Value {
    json!({"kind": trigger, "why": trigger.describe()})
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::engine::JevDialog;

    fn run(status: JevStatus, cause: Option<JevStopCause>, elements: usize) -> JevRunResult {
        let mut result = JevRunResult::before_start(status, "https://a.test/", Duration::ZERO, "x");
        result.stopped_because = None;
        result.stop_cause = cause;
        result.observed_elements = elements;
        result
    }

    #[test]
    fn statuses_that_fall_back() {
        use JevStopCause as C;
        let blocked = |cause| trigger(&run(JevStatus::Blocked, Some(cause), 12));
        assert_eq!(blocked(C::ModelBlocked), Some(Trigger::ModelBlocked));
        assert_eq!(blocked(C::Stalled), Some(Trigger::Stalled));
        assert_eq!(blocked(C::Covered), Some(Trigger::Covered));
        assert_eq!(blocked(C::DoneAfterCovered), Some(Trigger::Covered));
        assert_eq!(
            trigger(&run(JevStatus::Blocked, Some(C::ModelBlocked), 0)),
            Some(Trigger::NothingToActOn)
        );
        assert_eq!(
            trigger(&run(JevStatus::BudgetExceeded, Some(C::Budget), 40)),
            Some(Trigger::Budget)
        );
    }

    #[test]
    fn statuses_that_never_fall_back() {
        use JevStopCause as C;
        for status in [
            JevStatus::Done,
            JevStatus::NeedsInput,
            JevStatus::NeedsConfirmation,
            JevStatus::AccessDenied,
            JevStatus::TimedOut,
            JevStatus::Unavailable,
            JevStatus::Error,
        ] {
            assert_eq!(
                trigger(&run(status, Some(C::Stopped), 5)),
                None,
                "{status:?}"
            );
            assert_eq!(trigger(&run(status, None, 5)), None, "{status:?}");
        }
        for cause in [C::OutsideScope, C::NotLoaded, C::Stopped, C::Budget] {
            assert_eq!(
                trigger(&run(JevStatus::Blocked, Some(cause), 5)),
                None,
                "{cause:?}"
            );
        }
        // A budget status an embedder or harness chose is not Jev's budget.
        assert_eq!(
            trigger(&run(JevStatus::BudgetExceeded, Some(C::Stopped), 5)),
            None
        );
        // A page that did not load (before the loop) never falls back.
        let not_loaded = JevRunResult::before_start(
            JevStatus::Blocked,
            "https://a.test/",
            Duration::ZERO,
            "could not load (net::ERR_NAME_NOT_RESOLVED)",
        );
        assert_eq!(trigger(&not_loaded), None);
    }

    #[test]
    fn a_declined_confirm_is_the_callers_to_answer() {
        let mut result = run(JevStatus::Blocked, Some(JevStopCause::Stalled), 5);
        assert!(trigger(&result).is_some());
        result.stopped_because = Some(
            "The page asked \"Delete?\" in a confirm dialog and Jev dismissed it: Jev never \
             accepts a confirm or answers a prompt."
                .into(),
        );
        assert_eq!(trigger(&result), None);
        let mut result = run(JevStatus::Blocked, Some(JevStopCause::ModelBlocked), 5);
        let mut record = crate::engine::JevActionRecord::for_tests("click", "Delete");
        record.dialogs = vec![JevDialog {
            kind: "confirm".into(),
            message: "Delete?".into(),
            accepted: false,
        }];
        result.actions.push(record);
        assert_eq!(trigger(&result), None);
    }
}
