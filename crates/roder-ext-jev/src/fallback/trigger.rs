//! When a run falls back: Jev could not progress, and a different driver
//! might. It ended `blocked` because the model answered BLOCKED, three steps
//! changed nothing, its targets stayed covered, it answered DONE after a
//! covered attempt, it went round in circles (the same control a fourth time
//! on the same page, or six waits that changed nothing), or the page never
//! held still for it; the page offered nothing Jev can act on; its budget
//! ran out short of the goal; or its decision service kept sending replies
//! Jev could not use.
//!
//! That last one is the only `error` that falls back. The service answered
//! and was billed, but its replies failed validation, were refusals, or
//! could not be decoded, even after Jev asked again twice
//! ([`JevStopCause::DecisionUnusable`]); a frontier model with the full
//! browser tools may finish what Jev's decision service could not decide.
//! The owner decided so. It is the cause that routes, never the status alone,
//! so every other `error` stays a stop.
//!
//! Never when a different driver does not fix the cause, and never as a way
//! around a stop that protects the user: `needs_input` (a value the goal
//! lacks), `needs_confirmation` (the irreversible-action gate),
//! `access_denied` (a bot wall, a CAPTCHA, a refusal), a page that did not
//! load, a page outside the allowed origins, a confirm or prompt Jev
//! declined, a provider that could not be reached, a key, billing or other
//! HTTP refusal, any other error, or a timeout that has spent the call's
//! time.

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
    /// It went round in circles: the same control chosen a fourth time on a
    /// page that looked the same, or six waits in a row that changed
    /// nothing.
    Looped,
    /// The page kept changing under its decisions: three went stale in a
    /// row with nothing new on the page.
    Unsettled,
    /// The page offered nothing Jev can act on.
    NothingToActOn,
    /// The action or model-call budget ran out.
    Budget,
    /// The decision service kept sending replies Jev could not use (the
    /// run ended `error` after asking again twice).
    DecisionUnusable,
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
            Self::Looped => {
                "Jev went round in circles, repeating itself on a page that did not change"
            }
            Self::Unsettled => {
                "the page kept changing under Jev's decisions, so each one went stale"
            }
            Self::NothingToActOn => {
                "the page offered nothing Jev can act on (it cannot use canvas drawings, drags, \
                 hover-only menus or coordinates)"
            }
            Self::Budget => "Jev ran out of its action budget short of the goal",
            Self::DecisionUnusable => "the decision service kept sending replies Jev could not use",
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
                JevStopCause::Looped => Trigger::Looped,
                JevStopCause::Unsettled => Trigger::Unsettled,
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
        // The decision service kept sending replies Jev could not use. A
        // dialog Jev declined is still the caller's to answer, as above.
        JevStatus::Error if result.stop_cause == Some(JevStopCause::DecisionUnusable) => {
            match declined_a_dialog(result) {
                true => None,
                false => Some(Trigger::DecisionUnusable),
            }
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
        assert_eq!(blocked(C::Looped), Some(Trigger::Looped));
        assert_eq!(blocked(C::Unsettled), Some(Trigger::Unsettled));
        assert_eq!(
            trigger(&run(JevStatus::Blocked, Some(C::ModelBlocked), 0)),
            Some(Trigger::NothingToActOn)
        );
        assert_eq!(
            trigger(&run(JevStatus::Blocked, Some(C::Looped), 0)),
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
        // An error that is not unusable replies (a provider that could not be
        // reached, a key or billing refusal, anything without that cause)
        // never falls back, whatever else the run did.
        for cause in [None, Some(C::Stopped), Some(C::Budget)] {
            assert_eq!(
                trigger(&run(JevStatus::Error, cause, 40)),
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
        // Going round in circles after a declined confirm is the same question.
        result.stop_cause = Some(JevStopCause::Looped);
        assert_eq!(trigger(&result), None);
        result.stop_cause = Some(JevStopCause::Unsettled);
        assert_eq!(trigger(&result), None);
    }

    #[test]
    fn the_loop_caps_are_blocked_causes_and_nothing_else_falls_back_on_them() {
        use JevStopCause as C;
        for cause in [C::Looped, C::Unsettled] {
            // Only a blocked run falls back on them; a timeout that happened
            // to follow a loop, an error or a budget status does not.
            for status in [
                JevStatus::Done,
                JevStatus::TimedOut,
                JevStatus::Error,
                JevStatus::BudgetExceeded,
                JevStatus::NeedsInput,
            ] {
                assert_eq!(trigger(&run(status, Some(cause), 5)), None, "{status:?}");
            }
        }
        assert_ne!(Trigger::Looped.describe(), Trigger::Unsettled.describe());
        assert_eq!(
            recorded(Trigger::Looped)["kind"],
            serde_json::json!("looped")
        );
        assert_eq!(
            recorded(Trigger::Unsettled)["kind"],
            serde_json::json!("unsettled")
        );
    }

    #[test]
    fn an_error_after_unusable_replies_falls_back() {
        let error = |elements| {
            run(
                JevStatus::Error,
                Some(JevStopCause::DecisionUnusable),
                elements,
            )
        };
        assert_eq!(trigger(&error(40)), Some(Trigger::DecisionUnusable));
        // The cause is the service's, not the page's: a page with nothing on
        // it is still the service's failure.
        assert_eq!(trigger(&error(0)), Some(Trigger::DecisionUnusable));
        assert_eq!(
            recorded(Trigger::DecisionUnusable),
            serde_json::json!({
                "kind": "decision_unusable",
                "why": "the decision service kept sending replies Jev could not use",
            })
        );
        // Only an error ends this way: the same cause on another status is
        // not the exhausted-asks stop.
        for status in [
            JevStatus::Done,
            JevStatus::TimedOut,
            JevStatus::Unavailable,
            JevStatus::BudgetExceeded,
            JevStatus::NeedsInput,
            JevStatus::NeedsConfirmation,
            JevStatus::AccessDenied,
        ] {
            let result = run(status, Some(JevStopCause::DecisionUnusable), 40);
            assert_eq!(trigger(&result), None, "{status:?}");
        }
    }

    #[test]
    fn a_declined_confirm_stops_the_fallback_after_unusable_replies_too() {
        let mut result = run(JevStatus::Error, Some(JevStopCause::DecisionUnusable), 5);
        assert!(trigger(&result).is_some());
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
