use super::*;
use serde_json::json;

fn page(url: &str, actions: Value) -> Value {
    json!({"url": url, "actions": actions})
}

fn summary(before: &Value, after: &Value) -> String {
    effect(before, after, Landing::default()).summary
}

/// A ticket-type picker: its trigger, open, with its two options.
fn picker(value: &str, open: bool) -> Value {
    let mut actions = vec![json!({"node": 1, "kind": "click", "role": "combobox",
        "label": "Ticket type", "value": value, "expanded": open.to_string()})];
    if open {
        actions.push(
            json!({"node": 2, "kind": "click", "role": "option", "label": "One way", "value": ""}),
        );
        actions.push(json!({"node": 3, "kind": "click", "role": "option", "label": "Round trip", "value": ""}));
    }
    page("https://a.test/", Value::Array(actions))
}

#[test]
fn a_menu_closing_without_a_new_value_set_nothing() {
    let before = picker("Round trip", true);
    let chosen = before["actions"][1].clone();
    let done = effect(
        &before,
        &picker("Round trip", false),
        Landing {
            chosen: Some(&chosen),
            ..Landing::default()
        },
    );
    assert!(!done.set_something);
    assert_eq!(
        done.summary,
        "changed Ticket type expanded: true -> false; \
         removed 2 controls: One way, Round trip; One way set no value"
    );
    // The same click that set the value did take.
    let done = effect(
        &before,
        &picker("One way", false),
        Landing {
            chosen: Some(&chosen),
            ..Landing::default()
        },
    );
    assert!(done.set_something);
    assert!(
        done.summary
            .starts_with("changed Ticket type value: Round trip -> One way;"),
        "{}",
        done.summary
    );
}

#[test]
fn a_chosen_suggestion_that_leaves_the_typed_text_counts() {
    let before = page(
        "https://a.test/",
        json!([
            {"node": 1, "kind": "fill", "role": "combobox", "label": "Where from?", "value": "London"},
            {"node": 2, "kind": "click", "role": "option", "label": "London, United Kingdom", "value": ""},
            {"node": 3, "kind": "click", "role": "option", "label": "Londrina, Brazil", "value": ""},
        ]),
    );
    let after = page(
        "https://a.test/",
        json!([
            {"node": 1, "kind": "fill", "role": "combobox", "label": "Where from?", "value": "London"},
        ]),
    );
    let chosen = before["actions"][1].clone();
    let done = effect(
        &before,
        &after,
        Landing {
            chosen: Some(&chosen),
            ..Landing::default()
        },
    );
    assert!(done.set_something, "{done:?}");
    assert_eq!(
        done.summary,
        "removed 2 controls: London, United Kingdom, Londrina, Brazil"
    );
    // Without the choice, a list closing is all it says.
    assert!(!effect(&before, &after, Landing::default()).set_something);
}

#[test]
fn an_action_that_set_nothing_names_the_fields_the_form_still_needs() {
    let form = |value: &str, blocking: bool| {
        page(
            "https://a.test/",
            json!([
                {"node": 1, "kind": "click", "role": "button", "label": "Search", "value": ""},
                {"node": 2, "kind": "fill", "role": "textbox", "label": "Return", "value": value,
                 "blocking": blocking},
                // Offscreen fields the form needs are named too.
                {"node": 3, "kind": "fill", "role": "textbox", "label": "Email", "value": "",
                 "blocking": true, "offscreen": true},
            ]),
        )
    };
    let empty = form("", true);
    assert_eq!(
        summary(&empty, &empty),
        "nothing visible changed; fields the form still needs: 2 controls: Return, Email"
    );
    // A value set is progress: what the form still needs is not listed.
    let filled = form("Oct 23", false);
    assert_eq!(
        summary(&empty, &filled),
        "changed Return value: empty -> Oct 23"
    );
}

#[test]
fn only_the_acted_forms_fields_are_named_as_still_needed() {
    let trip = json!({"id": 7});
    let observation = page(
        "https://a.test/",
        json!([
            {"node": 1, "kind": "click", "role": "button", "label": "Search", "form": trip},
            {"node": 2, "kind": "fill", "role": "textbox", "label": "Return", "value": "",
             "blocking": true, "form": trip},
            // A footer's newsletter box is not why Search did nothing.
            {"node": 3, "kind": "fill", "role": "textbox", "label": "Newsletter", "value": "",
             "blocking": true, "form": {"id": 9}},
            {"node": 4, "kind": "fill", "role": "textbox", "label": "Coupon", "value": "",
             "blocking": true, "form": null},
        ]),
    );
    let form = json!(7);
    let scoped = Landing {
        form: Some(&form),
        ..Landing::default()
    };
    assert_eq!(
        effect(&observation, &observation, scoped).summary,
        "nothing visible changed; fields the form still needs: 1 control: Return"
    );
    // A control in no form names every field the page still needs.
    assert_eq!(
        summary(&observation, &observation),
        "nothing visible changed; fields the form still needs: 3 controls: Return, Newsletter, \
         Coupon"
    );
}

#[test]
fn an_error_status_after_a_navigation_is_named() {
    let before = page("https://a.test/", json!([]));
    let after = page("https://a.test/missing", json!([]));
    let landed = |status| {
        effect(
            &before,
            &after,
            Landing {
                status,
                ..Landing::default()
            },
        )
        .summary
    };
    assert_eq!(
        landed(Some(404)),
        "went to https://a.test/missing; HTTP 404: destination did not load successfully"
    );
    assert_eq!(landed(Some(200)), "went to https://a.test/missing");
    assert_eq!(landed(None), "went to https://a.test/missing");
}

#[test]
fn only_a_click_on_a_choice_chooses() {
    assert!(chooses(&json!({"kind": "click", "role": "option"})));
    assert!(chooses(&json!({"kind": "click", "role": "checkbox"})));
    assert!(!chooses(&json!({"kind": "click", "role": "button"})));
    assert!(!chooses(&json!({"kind": "fill", "role": "combobox"})));
}

#[test]
fn a_secret_field_never_shows_a_choice() {
    let field = Control::of(&json!({"node": 1, "kind": "fill", "role": "textbox",
        "label": "Code", "value": "", "filled": true}))
    .unwrap();
    assert!(!field.shows("filled"));
}

#[test]
fn nothing_changed_says_so() {
    let before = page(
        "https://a.test/",
        json!([
            {"node": 1, "kind": "click", "role": "button", "label": "Retry", "value": ""},
        ]),
    );
    assert_eq!(summary(&before, &before.clone()), "nothing visible changed");
}

#[test]
fn a_navigation_and_shown_and_removed_controls_are_named() {
    let before = page(
        "https://a.test/list",
        json!([
            {"node": 1, "kind": "click", "role": "link", "label": "Open report", "value": ""},
            {"node": 2, "kind": "click", "role": "link", "label": "Help", "value": ""},
        ]),
    );
    let after = page(
        "https://a.test/report",
        json!([
            {"node": 2, "kind": "click", "role": "link", "label": "Help", "value": ""},
            {"node": 3, "kind": "click", "role": "button", "label": "Approve", "value": ""},
            {"node": 4, "kind": "click", "role": "button", "label": "Reject", "value": ""},
            // Offscreen controls and scroll boxes are not news.
            {"node": 5, "kind": "click", "role": "button", "label": "Far", "offscreen": true},
        ]),
    );
    assert_eq!(
        summary(&before, &after),
        "went to https://a.test/report; showed 2 controls: Approve, Reject; \
         removed 1 control: Open report"
    );
}

#[test]
fn value_and_state_changes_are_listed_up_to_four() {
    let before = page(
        "https://a.test/",
        json!([
            {"node": 1, "kind": "fill", "role": "textbox", "label": "Name", "value": ""},
            {"node": 1, "kind": "click", "role": "textbox", "label": "Open Name", "value": ""},
            {"node": 2, "kind": "select", "role": "combobox", "label": "Size → Small",
             "current_value": "Small"},
            {"node": 2, "kind": "select", "role": "combobox", "label": "Size → Large",
             "current_value": "Small"},
            {"node": 3, "kind": "click", "role": "checkbox", "label": "Gift", "checked": "false"},
            {"node": 4, "kind": "click", "role": "button", "label": "Menu", "expanded": "false"},
            {"node": 5, "kind": "click", "role": "button", "label": "Count 1", "value": ""},
        ]),
    );
    let after = page(
        "https://a.test/",
        json!([
            {"node": 1, "kind": "fill", "role": "textbox", "label": "Name", "value": "Ada"},
            {"node": 2, "kind": "select", "role": "combobox", "label": "Size → Small",
             "current_value": "Large"},
            {"node": 3, "kind": "click", "role": "checkbox", "label": "Gift", "checked": "true"},
            {"node": 4, "kind": "click", "role": "button", "label": "Menu", "expanded": "true"},
            {"node": 5, "kind": "click", "role": "button", "label": "Count 2", "value": ""},
        ]),
    );
    assert_eq!(
        summary(&before, &after),
        "changed Name value: empty -> Ada; Size value: Small -> Large; \
         Gift checked: false -> true; Menu expanded: false -> true and 1 more"
    );
}

#[test]
fn a_secret_field_is_reported_filled_never_by_its_value() {
    let field = |filled: bool| {
        page(
            "https://a.test/",
            json!([{"node": 1, "kind": "fill", "role": "textbox", "label": "Password",
                    "value": "", "input_type": "password", "filled": filled}]),
        )
    };
    assert_eq!(
        summary(&field(false), &field(true)),
        "changed Password value: empty -> filled"
    );
}

#[test]
fn a_replaced_control_is_matched_by_its_role_and_label() {
    let before = page(
        "https://a.test/",
        json!([
            {"node": 1, "kind": "fill", "role": "combobox", "label": "From", "value": "Lon"},
        ]),
    );
    let after = page(
        "https://a.test/",
        json!([
            {"node": 9, "kind": "fill", "role": "combobox", "label": "From", "value": "London"},
        ]),
    );
    assert_eq!(
        summary(&before, &after),
        "changed From value: Lon -> London"
    );
}

/// After a navigation, node 1 is another document's first control: it is
/// not the old node 1 with a new label.
#[test]
fn node_ids_are_not_paired_across_documents() {
    let mut before = page(
        "https://a.test/cart",
        json!([
            {"node": 1, "kind": "click", "role": "button", "label": "Checkout", "value": ""},
            {"node": 2, "kind": "click", "role": "link", "label": "Help", "value": ""},
        ]),
    );
    before["page_key"] = json!([1000.5, "https://a.test/cart"]);
    let mut after = page(
        "https://a.test/pay",
        json!([
            {"node": 1, "kind": "fill", "role": "textbox", "label": "Card", "value": ""},
            {"node": 2, "kind": "click", "role": "link", "label": "Help", "value": "x"},
        ]),
    );
    after["page_key"] = json!([2000.25, "https://a.test/pay"]);
    assert_eq!(
        summary(&before, &after),
        "went to https://a.test/pay; changed Help value: empty -> x; \
         showed 1 control: Card; removed 1 control: Checkout"
    );
    // A tab the action opened is another document too.
    let tabbed = page(
        "https://a.test/cart",
        json!([
            {"node": 1, "kind": "click", "role": "button", "label": "Print", "value": ""},
        ]),
    );
    let mut opened = tabbed.clone();
    opened["opened_tab"] = json!(true);
    opened["actions"][0]["label"] = json!("Close");
    assert_eq!(
        summary(&tabbed, &opened),
        "opened a new tab, now active; showed 1 control: Close; removed 1 control: Print"
    );
    // The same document keeps pairing by node.
    let mut same = before.clone();
    same["actions"][0]["label"] = json!("Checkout (2)");
    assert_eq!(
        summary(&before, &same),
        "changed Checkout label: Checkout -> Checkout (2)"
    );
}

#[test]
fn a_new_tab_is_named() {
    let before = page("https://a.test/list", json!([]));
    let mut after = page("https://a.test/report", json!([]));
    after["opened_tab"] = json!(true);
    assert_eq!(
        summary(&before, &after),
        "went to https://a.test/report; opened a new tab, now active"
    );
}
