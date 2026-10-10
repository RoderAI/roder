//! A checkbox's or radio's state, from the page's observation into the
//! result's controls and the text's options. The snapshot reports a native
//! one as `checked: "true"` or `"false"` next to its HTML `value`, which is
//! "on" unless the page sets one; the result used to show that value and,
//! for a checked one, nothing else.

use serde_json::{Value, json};

use super::digest;
use super::digest::{BEGIN_PAGE, END_PAGE};
use crate::agent::controls;

const NOW: &str = "Mon 2026-09-28, 17:42 local time (America/Los_Angeles, UTC-07:00)";

fn observation() -> Value {
    json!({"actions": [
        {"id": "e1", "node": 1, "kind": "click", "role": "checkbox", "label": "Nonstop only",
         "checked": "false", "value": "on"},
        {"id": "e2", "node": 2, "kind": "click", "role": "checkbox", "label": "Free shipping",
         "checked": "true", "value": "on"},
        {"id": "e3", "node": 3, "kind": "click", "role": "radio", "label": "Express",
         "checked": "true", "value": "express"},
        {"id": "e4", "node": 4, "kind": "click", "role": "radio", "label": "Standard",
         "checked": "false", "value": "standard"},
        {"id": "e5", "node": 5, "kind": "click", "role": "switch", "label": "Alerts",
         "checked": "false", "value": ""},
        {"id": "e6", "node": 6, "kind": "click", "role": "button", "label": "Save", "value": ""},
        // A button whose own value says "checked" is not a toggle.
        {"id": "e7", "node": 7, "kind": "click", "role": "button", "label": "Continue",
         "value": "checked"},
    ]})
}

fn data() -> Value {
    json!({
        "status": "done", "url": "https://shop.test/", "title": "Shop", "elapsed_ms": 900,
        "visible_text": "Filters", "model_calls": 1, "actions": [],
        "controls": controls(&observation()),
    })
}

fn options_line(text: &str) -> &str {
    let page = &text[text.find(BEGIN_PAGE).unwrap()..text.find(END_PAGE).unwrap()];
    page.lines()
        .find(|line| line.starts_with("  (page):"))
        .unwrap_or_else(|| panic!("no options line: {text}"))
}

#[test]
fn a_native_checkbox_and_radio_report_checked_or_unchecked_not_their_html_value() {
    let controls = serde_json::to_value(controls(&observation())).unwrap();
    let state = |label: &str| {
        controls
            .as_array()
            .unwrap()
            .iter()
            .find(|control| control["label"] == label)
            .unwrap_or_else(|| panic!("no control {label}: {controls}"))
            .clone()
    };
    for (label, checked) in [
        ("Nonstop only", false),
        ("Free shipping", true),
        ("Express", true),
        ("Standard", false),
        ("Alerts", false),
    ] {
        let control = state(label);
        assert_eq!(control["checked"], json!(checked), "{label}: {control}");
        assert!(
            control.get("value").is_none(),
            "{label} shows its HTML value: {control}"
        );
    }
    // Not a toggle: no state, and its value is still its value.
    let button = state("Continue");
    assert!(button.get("checked").is_none(), "{button}");
    assert_eq!(button["value"], json!("checked"));
    assert!(state("Save").get("checked").is_none());
}

#[test]
fn the_options_say_checked_or_unchecked() {
    let text = digest(&data(), NOW);
    assert_eq!(
        options_line(&text),
        "  (page): Nonstop only [unchecked] · Free shipping [checked] · Express [checked] · \
         Standard [unchecked] · Alerts [unchecked] · Save · Continue"
    );
}

#[test]
fn a_fields_text_that_reads_checked_is_still_a_field() {
    let observation = json!({"actions": [
        {"id": "e1", "node": 1, "kind": "fill", "role": "textbox", "label": "Status",
         "value": "checked"},
    ]});
    let data = json!({"status": "done", "actions": [], "controls": controls(&observation)});
    let text = digest(&data, NOW);
    assert_eq!(options_line(&text), "  (page): Status [field: \"checked\"]");
}
