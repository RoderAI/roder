//! The pointer before a press, and what a click or a fill promises to open:
//! hover handlers run, a target that moves is followed, one that never stops
//! is refused, and the settle waits for an announced menu or for a search
//! field's suggestions.

use std::time::{Duration, Instant};

use serde_json::json;

use super::scripted::find;
use super::settle_tests::SETTLE_CEILING_MS;
use super::{act_on, timed_step};
use crate::engine::StaleObservation;

#[tokio::test]
async fn the_pointer_enters_the_target_before_the_press() {
    let harness = harness_or_skip!();
    let mut page = harness.open("pointer.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    act_on(&mut page, &observation, "click", "Reveal", None)
        .await
        .unwrap();
    assert_eq!(page.evaluate("window.revealed").await.unwrap(), json!(true));
}

#[tokio::test]
async fn a_target_that_moves_under_the_pointer_is_followed() {
    let harness = harness_or_skip!();
    let mut page = harness.open("pointer.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // The pointer arriving lets "Decoy" take the spot the hit test found; a
    // press there straight away would click it.
    act_on(&mut page, &observation, "click", "Shifty", None)
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("[window.shifty_clicks ?? 0, window.decoy_clicks ?? 0]")
            .await
            .unwrap(),
        json!([1, 0])
    );
}

#[tokio::test]
async fn a_target_that_never_stops_moving_is_not_pressed() {
    let harness = harness_or_skip!();
    let mut page = harness.open("pointer.html?moving=1").await.unwrap();
    let observation = page.observe().await.unwrap();
    let runaway = find(&observation, "click", "Runaway").unwrap().clone();
    let started = Instant::now();
    let error = page
        .act(&runaway, &observation, None, Duration::from_millis(100))
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    assert!(error.is::<StaleObservation>(), "{error:#}");
    assert_eq!(
        error.to_string(),
        "The target did not stop moving; nothing was clicked."
    );
    // Followed for a second, then given up.
    assert!(elapsed >= Duration::from_secs(1), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    assert_eq!(
        page.evaluate("window.runaway_clicked ?? false")
            .await
            .unwrap(),
        json!(false)
    );
}

#[tokio::test]
async fn a_menu_that_arrives_late_is_waited_for() {
    let harness = harness_or_skip!();
    let mut page = harness.open("menu.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let account = find(&observation, "click", "Account").unwrap().clone();
    let (next, timing) = timed_step(&mut page, &observation, &account, None)
        .await
        .unwrap();
    // The menu arrives 500 ms after the click; 200 ms of quiet came first.
    assert!(find(&next, "click", "Profile").is_some(), "{next:#}");
    assert!(timing.act_ms + timing.settle_ms >= 450.0, "{timing:?}");
    assert!(timing.settle_ms < SETTLE_CEILING_MS, "{timing:?}");
}

#[tokio::test]
async fn a_popup_that_never_opens_costs_at_most_the_wait() {
    let harness = harness_or_skip!();
    let mut page = harness.open("menu.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let help = find(&observation, "click", "Help").unwrap().clone();
    let (_, timing) = timed_step(&mut page, &observation, &help, None)
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.help_clicked").await.unwrap(),
        json!(true)
    );
    // 1.2 s for the popup, then the page is already quiet; the 2.5 s cap,
    // held with 250 ms of slack, bounds it on a loaded machine.
    assert!(timing.settle_ms >= 1150.0, "{timing:?}");
    assert!(timing.settle_ms < SETTLE_CEILING_MS, "{timing:?}");
}

#[tokio::test]
async fn a_search_fields_suggestions_are_waited_for() {
    let harness = harness_or_skip!();
    let mut page = harness.open("menu.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let search = find(&observation, "fill", "Search the site")
        .unwrap()
        .clone();
    // No combobox role: the field is a plain type=search, and its
    // suggestions arrive 600 ms after typing.
    let (next, timing) = timed_step(&mut page, &observation, &search, Some("Pri"))
        .await
        .unwrap();
    assert!(find(&next, "click", "Pricing").is_some(), "{next:#}");
    assert!(find(&next, "click", "Privacy policy").is_some(), "{next:#}");
    // The suggestions arrive 600 ms after the text, which the act typed;
    // 200 ms of quiet came long before.
    assert!(timing.act_ms + timing.settle_ms >= 550.0, "{timing:?}");
    assert!(timing.settle_ms < SETTLE_CEILING_MS, "{timing:?}");
}
