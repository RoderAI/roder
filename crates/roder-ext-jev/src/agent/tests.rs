//! Unit tests of the loop's stall rule, statuses and budgets.

use super::*;

fn entry(kind: &str, changed: Option<bool>) -> Value {
    json!({"kind": kind, "page_changed": changed})
}

#[test]
fn three_unchanged_actions_block_the_run() {
    let history = vec![
        entry("click", Some(false)),
        entry("click", Some(false)),
        entry("click", Some(false)),
    ];
    assert_eq!(stalled(&history), Some(JevStopCause::Stalled));
}

#[test]
fn a_wait_or_a_change_keeps_the_run_going() {
    assert!(
        stalled(&[
            entry("click", Some(false)),
            entry("wait", Some(false)),
            entry("click", Some(false)),
        ])
        .is_none()
    );
    assert!(
        stalled(&[
            entry("click", Some(false)),
            entry("click", Some(true)),
            entry("click", Some(false)),
        ])
        .is_none()
    );
    // A run that has not executed three actions yet cannot stall.
    assert!(stalled(&[entry("click", Some(false))]).is_none());
    assert!(stalled(&[]).is_none());
}

#[test]
fn three_covered_attempts_are_told_apart_from_a_stall() {
    let covered = || json!({"kind": "click", "page_changed": false, "covered": true});
    assert_eq!(
        stalled(&[covered(), covered(), covered()]),
        Some(JevStopCause::Covered)
    );
    assert_eq!(
        stalled(&[covered(), entry("click", Some(false)), covered()]),
        Some(JevStopCause::Stalled)
    );
}

#[test]
fn a_refused_cookie_banner_neither_counts_nor_breaks_a_stall() {
    let banner = || json!({"kind": COOKIE_BANNER, "page_changed": null});
    assert!(
        Some(JevStopCause::Stalled)
            == stalled(&[
                entry("click", Some(false)),
                banner(),
                entry("click", Some(false)),
                entry("click", Some(false)),
            ])
    );
    assert!(
        stalled(&[
            banner(),
            entry("click", Some(false)),
            entry("click", Some(false)),
        ])
        .is_none()
    );
}

#[test]
fn only_the_last_three_actions_matter() {
    let history = vec![
        entry("click", Some(true)),
        entry("click", Some(false)),
        entry("click", Some(false)),
        entry("click", Some(false)),
    ];
    assert_eq!(stalled(&history), Some(JevStopCause::Stalled));
}

#[test]
fn statuses_keep_upstream_names_and_only_ready_runs_on() {
    assert!(JevStatus::Done.stopped() && JevStatus::Blocked.stopped());
    assert!(!JevStatus::Ready.stopped());
    let serialized = [
        (JevStatus::Ready, "ready"),
        (JevStatus::Done, "done"),
        (JevStatus::Blocked, "blocked"),
        (JevStatus::BudgetExceeded, "budget_exceeded"),
        (JevStatus::TimedOut, "timed_out"),
        (JevStatus::NeedsInput, "needs_input"),
        (JevStatus::Unavailable, "unavailable"),
        (JevStatus::Error, "error"),
        (JevStatus::NeedsConfirmation, "needs_confirmation"),
    ];
    for (status, name) in serialized {
        assert_eq!(serde_json::to_value(status).unwrap(), json!(name));
        assert_eq!(status.stopped(), status != JevStatus::Ready);
    }
}

#[test]
fn a_stop_carries_its_status_and_anything_else_is_an_error() {
    let stop = |status| anyhow::Error::from(JevStop::new(status, "why"));
    assert_eq!(
        JevStop::status_of(&stop(JevStatus::Unavailable)),
        JevStatus::Unavailable
    );
    assert_eq!(stop(JevStatus::NeedsInput).to_string(), "why");
    // Context added on the way up keeps the status.
    let wrapped = stop(JevStatus::BudgetExceeded).context("while deciding");
    assert_eq!(JevStop::status_of(&wrapped), JevStatus::BudgetExceeded);
    assert_eq!(
        JevStop::status_of(&anyhow::anyhow!("boom")),
        JevStatus::Error
    );
    // A stop cannot claim success or keep the run going.
    assert_eq!(JevStop::status_of(&stop(JevStatus::Done)), JevStatus::Error);
    assert_eq!(
        JevStop::status_of(&stop(JevStatus::Ready)),
        JevStatus::Error
    );
}

#[test]
fn budgets_match_upstream() {
    use crate::prompts::MAX_STEPS;
    assert_eq!(MAX_STEPS, 60);
    assert_eq!(MAX_STEPS * 2, 120);
}
