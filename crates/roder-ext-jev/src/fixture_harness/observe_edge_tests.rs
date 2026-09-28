//! What the snapshot offers and reads on pages that used to fool it: an icon
//! drawn in a shadow root inside a button, strings cut inside an emoji, a
//! custom element's odd `value`, an app shell's scrolling pane, display:
//! contents, text directly in a shadow root and in slots, a transparent
//! native select, focusable things that are not clickable, editable boxes,
//! masked secrets (offered, never read), a very long select, and a frame
//! that navigated.

use serde_json::{Value, json};

use super::scripted::find;

/// Every action as "kind label".
fn offered(observation: &Value) -> Vec<String> {
    observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| {
            format!(
                "{} {}",
                action["kind"].as_str().unwrap_or_default(),
                action["label"].as_str().unwrap_or_default()
            )
        })
        .collect()
}

#[tokio::test]
async fn an_icon_drawn_in_a_shadow_root_inside_a_button_is_the_buttons_inside() {
    let harness = harness_or_skip!();
    let mut page = harness.open("icon-button.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let clicks = offered(&observation)
        .into_iter()
        .filter(|action| action.starts_with("click "))
        .collect::<Vec<_>>();
    assert_eq!(clicks, vec!["click Delete item"], "{observation:#}");
}

#[tokio::test]
async fn strings_cut_inside_an_emoji_or_holding_half_of_one_are_observed() {
    let harness = harness_or_skip!();
    let mut page = harness.open("surrogates.html").await.unwrap();
    let observation = page.observe().await.expect("observe the page");
    let text = observation["text"].as_str().unwrap();
    // The cut backs off to a whole character.
    assert!(text.starts_with(&"a".repeat(5999)), "{}", &text[..20]);
    assert_eq!(text.chars().count(), 5999, "{}", &text[5990..]);
    let card = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| {
            action["label"]
                .as_str()
                .unwrap_or_default()
                .starts_with("bbb")
        })
        .expect("the card")
        .clone();
    assert_eq!(card["label"], json!(format!("{}…", "b".repeat(98))));
    // Half an emoji the page wrote itself arrives as U+FFFD.
    assert!(
        find(&observation, "click", "Save \u{fffd}").is_some(),
        "{observation:#}"
    );
    let note = find(&observation, "fill", "Note").unwrap();
    assert_eq!(note["value"], json!("Draft \u{fffd}"));
    // And the freshness check reads the page the same way.
    assert!(page.fresh(&observation, None).await.unwrap());
}

#[tokio::test]
async fn a_custom_elements_object_value_does_not_stop_the_observation() {
    let harness = harness_or_skip!();
    let mut page = harness.open("custom-value.html").await.unwrap();
    let observation = page.observe().await.expect("observe the page");
    let knob = find(&observation, "click", "Turn up")
        .expect("the knob")
        .clone();
    assert_eq!(knob["value"], json!(""));
    let guard = &observation["guards"][knob["node"].to_string()];
    assert_eq!(guard[3], json!("[object Object]"), "{guard}");
    assert!(page.fresh(&observation, Some(&knob)).await.unwrap());
}

#[tokio::test]
async fn an_app_shells_pane_is_scrolled_to_reach_its_controls() {
    let harness = harness_or_skip!();
    let mut page = harness.open("app-shell.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let next = find(&observation, "click", "Next page").expect("the pager");
    assert_eq!(next["offscreen"], json!(true));
    let far = find(&observation, "click", "Open mail 40").expect("a mail below the fold");
    assert_eq!(far["offscreen"], json!(true));
    // The document does not scroll, so no page scroll is offered.
    assert!(
        !observation["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["id"] == "scroll_down")
    );
}

#[tokio::test]
async fn display_contents_links_and_text_are_offered_and_read() {
    let harness = harness_or_skip!();
    let mut page = harness.open("contents.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let card = find(&observation, "click", "Blue lamp").expect("the card link");
    assert_eq!(card["role"], json!("link"));
    assert!(card["rect"]["w"].as_f64().unwrap() > 100.0, "{card}");
    let text = observation["text"].as_str().unwrap();
    assert!(text.contains("Price: 12 EUR"), "{text}");
}

#[tokio::test]
async fn shadow_text_is_read_in_place_with_its_slots() {
    let harness = harness_or_skip!();
    let mut page = harness.open("shadow-text.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    assert_eq!(observation["text"], json!("Total:\n42\nEUR\ntoday"));
}

#[tokio::test]
async fn a_transparent_native_select_is_offered() {
    let harness = harness_or_skip!();
    let mut page = harness.open("styled-select.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    assert!(
        find(&observation, "select", "Sort by → Cheapest").is_some(),
        "{:?}",
        offered(&observation)
    );
}

#[tokio::test]
async fn a_tabindex_alone_does_not_make_a_slider_panel_or_code_a_button() {
    let harness = harness_or_skip!();
    let mut page = harness.open("focusables.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let clicks = offered(&observation)
        .into_iter()
        .filter(|action| action.starts_with("click "))
        .collect::<Vec<_>>();
    assert_eq!(clicks, vec!["click Weekly digest"], "{observation:#}");
}

#[tokio::test]
async fn editable_boxes_are_not_named_by_what_they_hold() {
    let harness = harness_or_skip!();
    let mut page = harness.open("editables.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let message = find(&observation, "fill", "Message").expect("the message box");
    assert_eq!(message["value"], json!("Hello Bob, the draft is attached"));
    let search = find(&observation, "fill", "Search notes").expect("the ARIA textbox");
    assert_eq!(search["value"], json!("quarterly plan"));
    assert!(
        !offered(&observation).iter().any(
            |action| action.contains("draft is attached") || action.ends_with("quarterly plan")
        ),
        "{:?}",
        offered(&observation)
    );
}

#[tokio::test]
async fn masked_secrets_are_offered_but_never_read() {
    let harness = harness_or_skip!();
    let mut page = harness.open("masked.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    assert!(find(&observation, "fill", "Name").is_some());
    let everything = observation.to_string();
    assert!(everything.contains("Ada Lovelace"));
    assert!(!everything.contains("SECRET"), "{observation:#}");
    for (label, kind) in [
        ("PIN", "password"),
        ("Password", "password"),
        ("Code", "one-time-code"),
    ] {
        let field = find(&observation, "fill", label).unwrap_or_else(|| panic!("{label}"));
        assert_eq!(field["input_type"], json!(kind), "{label}");
        assert_eq!(field["filled"], json!(true), "{label}");
        assert_eq!(field["value"], json!(""), "{label}");
    }
}

#[tokio::test]
async fn a_long_select_counts_once_against_the_caps() {
    let harness = harness_or_skip!();
    let mut page = harness.open("long-select.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    assert!(
        find(&observation, "click", "Submit").is_some(),
        "Submit was dropped"
    );
    assert!(find(&observation, "fill", "City").is_some());
    assert!(find(&observation, "select", "Country → Country 300").is_some());
}

#[tokio::test]
async fn a_navigated_frames_old_nodes_are_let_go() {
    let harness = harness_or_skip!();
    let mut page = harness.open("frames.html").await.unwrap();
    page.observe().await.unwrap();
    page.evaluate("document.getElementById('form').src = 'frame-button.html?label=Reload'")
        .await
        .unwrap();
    // Wait for the frame's new document.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let observation = page.observe().await.unwrap();
        if find(&observation, "click", "Reload").is_some() {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "{observation:#}");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let orphans = page
        .evaluate(
            "[...window.__jevFast.nodes.values()].filter(e => !e.ownerDocument.defaultView).length",
        )
        .await
        .unwrap();
    assert_eq!(orphans, json!(0));
}
