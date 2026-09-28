//! Naming and freshness details of `snapshot.js` on a real DOM.

use std::time::Duration;

use serde_json::json;

use super::scripted::find;

#[tokio::test]
async fn script_and_style_text_never_names_a_control() {
    let harness = harness_or_skip!();
    let mut page = harness.open("names.html").await.unwrap();
    let observation = page.observe().await.unwrap();

    let card = find(&observation, "click", "Trail Runner");
    assert!(card.is_some(), "{:#}", observation["actions"]);
}

#[tokio::test]
async fn aria_labelledby_resolves_inside_a_shadow_root_first() {
    let harness = harness_or_skip!();
    let mut page = harness.open("names.html").await.unwrap();
    // The snapshot installs the naming function it uses.
    page.observe().await.unwrap();
    let name = |id: &str| {
        format!(
            "window.__jevFast.name(document.getElementById('host').shadowRoot.getElementById('{id}'))"
        )
    };

    assert_eq!(
        page.evaluate(&name("inner")).await.unwrap(),
        json!("Shadow label")
    );
    // An id the shadow root lacks falls back to the document.
    assert_eq!(
        page.evaluate(&name("fallback")).await.unwrap(),
        json!("Outside label")
    );
}

#[tokio::test]
async fn hydrating_aria_disabled_to_false_keeps_a_decision_fresh() {
    let harness = harness_or_skip!();
    let mut page = harness.open("names.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let button = find(&observation, "click", "Buy now")
        .expect("button observed")
        .clone();
    let set = |value: &str| {
        format!("document.getElementById('hydrate').setAttribute('aria-disabled', '{value}')")
    };

    page.evaluate(&set("false")).await.unwrap();
    assert!(page.fresh(&observation, Some(&button)).await.unwrap());
    page.act(&button, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(page.evaluate("window.clicks ?? 0").await.unwrap(), json!(1));

    // Actually disabling it still makes the decision stale.
    page.evaluate(&set("true")).await.unwrap();
    assert!(!page.fresh(&observation, Some(&button)).await.unwrap());
}

#[tokio::test]
async fn a_styled_checkbox_is_offered_and_toggled_through_its_label() {
    let harness = harness_or_skip!();
    let mut page = harness.open("filters.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let free = find(&observation, "click", "Free shipping")
        .unwrap_or_else(|| panic!("{:#}", observation["actions"]))
        .clone();
    assert_eq!(free["role"], "checkbox");
    assert_eq!(free["checked"], "false");
    // Offered once: the transparent inputs themselves are not.
    let checkboxes = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["role"] == "checkbox")
        .count();
    assert_eq!(checkboxes, 2, "{:#}", observation["actions"]);

    let (_, next) = super::act_on(&mut page, &observation, "click", "Free shipping", None)
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("document.getElementById('free').checked")
            .await
            .unwrap(),
        json!(true)
    );
    assert_eq!(
        find(&next, "click", "Free shipping").unwrap()["checked"],
        "true"
    );
    assert!(next["text"].as_str().unwrap().contains("2 products"));
}

#[tokio::test]
async fn an_anchor_without_href_that_handles_clicks_is_a_button() {
    let harness = harness_or_skip!();
    let mut page = harness.open("filters.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let sort = find(&observation, "click", "Sort by price").unwrap();
    assert_eq!(sort["role"], "button");
    super::act_on(&mut page, &observation, "click", "Sort by price", None)
        .await
        .unwrap();
    assert_eq!(page.evaluate("window.sorted").await.unwrap(), json!(true));
}

#[tokio::test]
async fn a_visible_checkbox_is_still_offered_itself() {
    let harness = harness_or_skip!();
    let mut page = harness.open("basic.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // The label wraps a checkbox anyone can see, so only the input counts.
    let newsletter = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["label"] == "Newsletter")
        .collect::<Vec<_>>();
    assert_eq!(newsletter.len(), 1, "{newsletter:#?}");
    assert_eq!(newsletter[0]["checked"], "false");
}
