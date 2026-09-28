//! Acting on the controls `observe_edge_tests` covers, and on pages whose
//! suggestions update late, whose fields reformat what is typed, or whose
//! page scroll sat over another scroller.

use serde_json::json;

use super::act_on;
use crate::engine::JevActOutcome;

#[tokio::test]
async fn a_button_whose_icon_draws_in_a_shadow_root_is_clicked() {
    let harness = harness_or_skip!();
    let mut page = harness.open("icon-button.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (outcome, _) = act_on(&mut page, &observation, "click", "Delete item", None)
        .await
        .expect("the button is not covered by its own icon");
    assert_eq!(outcome, JevActOutcome::done());
    assert_eq!(page.evaluate("window.deleted").await.unwrap(), json!(1));
}

#[tokio::test]
async fn a_display_contents_link_is_clicked() {
    let harness = harness_or_skip!();
    let mut page = harness.open("contents.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (_, next) = act_on(&mut page, &observation, "click", "Blue lamp", None)
        .await
        .unwrap();
    assert_eq!(next["title"], json!("Basic controls"), "{next:#}");
}

#[tokio::test]
async fn a_control_below_an_app_shells_fold_is_clicked() {
    let harness = harness_or_skip!();
    let mut page = harness.open("app-shell.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    act_on(&mut page, &observation, "click", "Open mail 40", None)
        .await
        .unwrap();
    assert_eq!(page.evaluate("window.opened").await.unwrap(), json!(40));
}

#[tokio::test]
async fn a_transparent_native_select_is_chosen() {
    let harness = harness_or_skip!();
    let mut page = harness.open("styled-select.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (outcome, _) = act_on(
        &mut page,
        &observation,
        "select",
        "Sort by → Cheapest",
        None,
    )
    .await
    .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    assert_eq!(
        page.evaluate("document.getElementById('shown').textContent")
            .await
            .unwrap(),
        json!("Cheapest")
    );
}

/// A list still showing the last query's options is not the answer to the
/// new one: the settle waits for the options to change (here 500 ms after
/// typing), and a hidden listbox elsewhere never counts.
#[tokio::test]
async fn suggestions_left_from_the_last_query_are_waited_past() {
    let harness = harness_or_skip!();
    let mut page = harness.open("debounce.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (outcome, next) = act_on(&mut page, &observation, "fill", "Fruit", Some("B"))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    let text = next["text"].as_str().unwrap();
    assert!(
        text.contains("Banana") && text.contains("Blueberry"),
        "{text}"
    );
    assert!(!text.contains("Apricot"), "{text}");
}

/// A field that formats what is typed kept it, and a datetime field that
/// drops zero seconds kept the same moment: neither is a refusal.
#[tokio::test]
async fn a_field_that_formats_the_typed_text_kept_it() {
    let harness = harness_or_skip!();
    let mut page = harness.open("formats.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (outcome, next) = act_on(&mut page, &observation, "fill", "Phone", Some("5551234567"))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    assert_eq!(
        page.evaluate("document.getElementById('phone').value")
            .await
            .unwrap(),
        json!("(555) 123-4567")
    );
    let (outcome, _) = act_on(
        &mut page,
        &next,
        "fill",
        "When",
        Some("2026-10-12T14:30:00"),
    )
    .await
    .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    assert_eq!(
        page.evaluate("document.getElementById('when').value")
            .await
            .unwrap(),
        json!("2026-10-12T14:30")
    );
    // A value the field does not keep is still refused.
    let observation = page.observe().await.unwrap();
    let (outcome, _) = act_on(&mut page, &observation, "fill", "When", Some("12/10/2026"))
        .await
        .unwrap();
    assert!(outcome.refused.is_some(), "{outcome:?}");
}

/// The page scroll moves the page, not a scroller that happens to sit where
/// a wheel used to be sent.
#[tokio::test]
async fn scrolling_the_page_moves_the_page() {
    let harness = harness_or_skip!();
    let mut page = harness.open("wheel.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let down = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["id"] == "scroll_down")
        .expect("a page scroll")
        .clone();
    page.act(
        &down,
        &observation,
        None,
        std::time::Duration::from_millis(100),
    )
    .await
    .unwrap();
    let after = page.observe().await.unwrap();
    assert_eq!(after["scroll"]["y"], json!(560), "{}", after["scroll"]);
    assert_eq!(
        page.evaluate("document.getElementById('map').scrollTop")
            .await
            .unwrap(),
        json!(0)
    );
}
