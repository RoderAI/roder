//! Setting a task up on real Chrome: the endpoint forms, a start page that
//! does not load, and a deadline that runs out before the loop starts. Every
//! test also checks that no tab is left behind.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::scripted::PlanDecider;
use crate::engine::{JevEngineConfig, JevRunResult, JevStatus};
use crate::page::LoadFailed;
use crate::runner::{Task, drive};

fn task(url: String, decider: Arc<PlanDecider>, timeout: Duration) -> Task {
    let started = tokio::time::Instant::now();
    Task {
        url,
        foreground: false,
        started,
        deadline: started + timeout,
        config: JevEngineConfig::new("setup goal", decider).with_wait(Duration::from_millis(100)),
    }
}

/// A loopback URL nothing listens on.
async fn closed_port_url() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    url
}

/// Drive a task that must stop before the loop: nothing decided, nothing
/// observed, and the tab gone again.
async fn stopped_before_the_loop(
    harness: &super::Harness,
    url: String,
    timeout: Duration,
) -> JevRunResult {
    let tabs = harness.page_targets().await;
    let decider = Arc::new(PlanDecider::new(Vec::new()));
    let result = drive(harness.endpoint(), task(url, decider.clone(), timeout))
        .await
        .unwrap();
    assert!(decider.chosen.lock().unwrap().is_empty(), "{result:#?}");
    assert_eq!(result.model_calls, 0);
    assert_eq!(result.observed_elements, 0);
    assert!(result.actions.is_empty());
    assert_eq!(
        harness.settled_page_targets(tabs).await,
        tabs,
        "a tab was left open"
    );
    result
}

#[tokio::test]
async fn a_websocket_endpoint_skips_the_version_lookup() {
    let harness = harness_or_skip!();
    let version: Value = reqwest::get(format!("{}/json/version", harness.endpoint()))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let websocket = version["webSocketDebuggerUrl"].as_str().unwrap();
    assert!(websocket.starts_with("ws://127.0.0.1:"), "{websocket}");
    let tabs = harness.page_targets().await;

    let decider = Arc::new(PlanDecider::new(Vec::new()));
    let url = harness.site.url("basic.html");
    let result = drive(websocket, task(url, decider, Duration::from_secs(20)))
        .await
        .unwrap();

    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.title, "Basic controls");
    assert_eq!(result.model_calls, 1);
    // A background task closes its tab when it ends.
    assert_eq!(harness.settled_page_targets(tabs).await, tabs);
}

#[tokio::test]
async fn a_refused_connection_is_blocked_as_unreachable_after_two_retries() {
    let harness = harness_or_skip!();
    let url = closed_port_url().await;
    let started = Instant::now();

    let result = stopped_before_the_loop(&harness, url.clone(), Duration::from_secs(20)).await;

    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("could not load: the site is unreachable (net::ERR_CONNECTION_REFUSED)")
    );
    assert_eq!(result.url, url);
    // Two retries, 0.5 s and 1 s apart.
    assert!(started.elapsed() >= Duration::from_millis(1500));
}

#[tokio::test]
async fn an_empty_response_is_retried() {
    let harness = harness_or_skip!();
    let url = harness.site.path_url("/fail/drop");
    let started = Instant::now();

    let result = stopped_before_the_loop(&harness, url, Duration::from_secs(20)).await;

    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("could not load (net::ERR_EMPTY_RESPONSE)")
    );
    // Chrome retries an empty response on its own, so the request count
    // only bounds it from below; the two waits show Jev's own retries.
    assert!(harness.site.failure_hits("/fail/drop") >= 3);
    assert!(started.elapsed() >= Duration::from_millis(1500));
}

#[tokio::test]
async fn an_abandoned_load_is_not_retried() {
    let harness = harness_or_skip!();
    let url = harness.site.path_url("/fail/no-content");

    let result = stopped_before_the_loop(&harness, url, Duration::from_secs(20)).await;

    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("could not load (net::ERR_ABORTED)")
    );
    assert_eq!(harness.site.failure_hits("/fail/no-content"), 1);
}

#[tokio::test]
async fn a_host_that_does_not_resolve_is_not_called_unreachable() {
    let harness = harness_or_skip!();
    // `.invalid` never resolves (RFC 6761).
    let url = "http://jev-fixture.invalid/".to_string();

    let result = stopped_before_the_loop(&harness, url, Duration::from_secs(20)).await;

    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("could not load (net::ERR_NAME_NOT_RESOLVED)")
    );
}

#[tokio::test]
async fn opening_a_page_that_fails_to_load_closes_its_tab() {
    let harness = harness_or_skip!();
    let tabs = harness.page_targets().await;

    let error = match harness.open_url(&closed_port_url().await).await {
        Ok(_) => panic!("a closed port loaded"),
        Err(error) => error,
    };

    assert!(error.is::<LoadFailed>(), "{error:#}");
    assert_eq!(harness.settled_page_targets(tabs).await, tabs);
}

#[tokio::test]
async fn a_page_still_loading_at_the_deadline_times_out_and_closes_its_tab() {
    let harness = harness_or_skip!();
    let url = harness.site.url("basic.html?delay=5000");
    let started = Instant::now();

    let result = stopped_before_the_loop(&harness, url, Duration::from_secs(1)).await;

    assert_eq!(result.status, JevStatus::TimedOut);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("Jev browser task timed out while loading the page")
    );
    // The page's own 15 s load wait no longer runs past the task's timeout.
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    assert!(result.elapsed_ms >= 1000, "{}", result.elapsed_ms);
}

#[tokio::test]
async fn a_deadline_already_passed_stops_before_connecting() {
    let harness = harness_or_skip!();
    let url = harness.site.url("basic.html");

    let result = stopped_before_the_loop(&harness, url, Duration::ZERO).await;

    assert_eq!(result.status, JevStatus::TimedOut);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("Jev browser task timed out while connecting to Chrome")
    );
}

/// A deadline that passes while the new tab is being attached to (or shown)
/// ends the task `timed_out`, and the tab it created is closed; it used to be
/// left open.
#[tokio::test]
async fn a_deadline_during_the_attach_closes_the_created_tab() {
    let harness = harness_or_skip!();
    let proxy = super::proxy::SlowProxy::start(
        harness.endpoint(),
        "Target.attachToTarget",
        Duration::from_secs(3),
    )
    .await;
    let tabs = harness.page_targets().await;
    let decider = Arc::new(PlanDecider::new(Vec::new()));
    let url = harness.site.url("basic.html");
    let result = drive(&proxy.url, task(url, decider, Duration::from_secs(1)))
        .await
        .unwrap();
    assert_eq!(result.status, JevStatus::TimedOut, "{result:#?}");
    assert_eq!(
        result.stopped_because.as_deref(),
        Some("Jev browser task timed out while opening a tab")
    );
    assert_eq!(
        harness.settled_page_targets(tabs).await,
        tabs,
        "the created tab was left open"
    );
}
