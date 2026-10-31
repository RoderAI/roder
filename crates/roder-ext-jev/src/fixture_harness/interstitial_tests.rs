//! Opening pages that clear themselves, on real pages: a challenge served
//! 403 that moves on after a few seconds, and a 503 that retries itself,
//! are watched until they do, so the run goes on from the page they lead
//! to; a challenge that never clears still ends `access_denied` before any
//! decision, and a plain "Access Denied" is not waited on. A start page
//! that stays blank is loaded once more.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::scripted::{PlanDecider, pick};
use crate::engine::JevStatus;

const RUN: Duration = Duration::from_secs(60);

fn getting_started() -> Arc<PlanDecider> {
    Arc::new(PlanDecider::new(vec![pick("click", "Getting started")]))
}

#[tokio::test]
async fn a_challenge_that_moves_on_is_waited_out() {
    let harness = harness_or_skip!();
    let result = harness
        .run(
            "challenge.html?status=403&after=2500",
            getting_started(),
            None,
            RUN,
        )
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.model_calls, 2, "{result:#?}");
    assert!(result.url.ends_with("/pages/challenge-cleared.html"));
    assert!(result.visible_text.contains("cargo install toolkit"));
}

#[tokio::test]
async fn a_503_that_retries_itself_is_waited_out() {
    let harness = harness_or_skip!();
    let result = harness
        .run(
            "challenge-cleared.html?unavailable_once=1&after=2000",
            getting_started(),
            None,
            RUN,
        )
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.model_calls, 2, "{result:#?}");
    assert_eq!(result.page.as_ref().unwrap().http_status, Some(200));
}

#[tokio::test]
async fn a_challenge_that_never_clears_ends_access_denied_after_the_wait() {
    let harness = harness_or_skip!();
    let started = Instant::now();
    let result = harness
        .run(
            "challenge.html?status=403&after=60000",
            getting_started(),
            None,
            RUN,
        )
        .await;
    assert_eq!(result.status, JevStatus::AccessDenied, "{result:#?}");
    assert_eq!(result.model_calls, 0);
    let reason = result.stopped_because.as_deref().unwrap();
    assert!(reason.contains("HTTP 403"), "{reason}");
    assert!(
        started.elapsed() >= Duration::from_secs(12),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_plain_refusal_is_not_waited_on() {
    let harness = harness_or_skip!();
    let started = Instant::now();
    let result = harness
        .run(
            "access-denied.html?status=403",
            getting_started(),
            None,
            RUN,
        )
        .await;
    assert_eq!(result.status, JevStatus::AccessDenied, "{result:#?}");
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_blank_start_page_is_loaded_once_more() {
    let harness = harness_or_skip!();
    let result = harness
        .run(
            "challenge-cleared.html?blank_once=1",
            getting_started(),
            None,
            RUN,
        )
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.model_calls, 2, "{result:#?}");
    assert_eq!(result.actions.len(), 1);
}
