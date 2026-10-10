use serde_json::json;

use super::*;
use crate::observed_render::{Budget, View, render};

fn snapshot(url: &str, text: &str, controls: Vec<Value>) -> Value {
    json!({
        "title": "Page", "url": url, "text": text, "controls": controls,
        "viewport": {"width": 1280, "height": 800, "scrollX": 0, "scrollY": 0}
    })
}

fn button(reference: &str, text: &str) -> Value {
    json!({"ref": reference, "tag": "button", "role": "button", "text": text})
}

#[test]
fn a_snapshot_control_is_labelled_the_way_a_person_would_name_it() {
    let page = Observed::from_snapshot(&snapshot(
        "https://a.test/",
        "",
        vec![
            json!({"ref": "c1", "tag": "input", "role": "textbox", "text": "typed value",
                "ariaName": "Email", "value": "typed value", "type": "text", "placeholder": "you@"}),
            json!({"ref": "c2", "tag": "input", "role": "textbox", "text": "", "placeholder": "Search"}),
            json!({"ref": "c3", "tag": "select", "role": "combobox", "text": "OneTwo",
                "name": "plan", "value": "two", "selector": "#plan"}),
            json!({"ref": "c4", "tag": "input", "role": "checkbox", "value": "on",
                "type": "checkbox", "ariaName": "Agree", "checked": true}),
            json!({"ref": "c5", "tag": "input", "role": "textbox", "type": "password",
                "ariaName": "Password", "value": "hunter22"}),
            json!({"ref": "c6", "tag": "button", "role": "button", "text": "Pay", "disabled": true}),
            json!({"tag": "button", "text": "no ref, so nothing to act on"}),
        ],
    ));
    let labels: Vec<_> = page
        .controls
        .iter()
        .map(|c| (c.reference.as_str(), c.label.as_str(), c.value.as_deref()))
        .collect();
    assert_eq!(
        labels,
        [
            ("c1", "Email", Some("typed value")),
            ("c2", "Search", None),
            ("c3", "plan", Some("two")),
            ("c4", "Agree", None),
            ("c5", "Password", None),
            ("c6", "Pay", None),
        ]
    );
    assert_eq!(page.controls[2].selector.as_deref(), Some("#plan"));
    assert_eq!(page.controls[0].selector, None, "only a select needs one");
    assert!(page.controls[4].secret && page.controls[5].disabled);
    assert_eq!(page.controls[3].checked, Some(true));
}

#[test]
fn forms_and_frames_are_one_line_each() {
    let mut page = snapshot("https://a.test/", "", vec![]);
    page["forms"] = json!([{
        "ref": "f1", "name": "invoiceSearch", "action": "https://a.test/search", "method": "get",
        "fields": [
            {"ref": "c1", "name": "q", "type": "search", "required": true, "label": "Search invoices"},
            {"ref": "c2", "name": "page", "type": "hidden"}
        ]
    }, {"ref": "f2", "fields": []}]);
    page["iframes"] = json!([
        {"index": 0, "src": "https://widgets.test/support", "crossOrigin": true},
        {"index": 1, "crossOrigin": false}
    ]);
    let page = Observed::from_snapshot(&page);
    assert_eq!(
        page.forms,
        [
            "form invoiceSearch GET https://a.test/search: c1 Search invoices (search, required), c2 page (hidden)",
            "form"
        ]
    );
    assert_eq!(
        page.frames,
        [
            "iframe 0 (cross-origin, not readable) https://widgets.test/support",
            "iframe 1 (same-origin)"
        ]
    );
}

#[test]
fn a_look_element_reads_like_a_snapshot_control() {
    let page = Observed::from_look(&json!({
        "url": "https://a.test/", "title": "A", "text": "Hello\n\n world",
        "viewport": {"w": 800, "h": 600, "scroll_y": 40, "page_h": 2000},
        "elements": [
            {"ref": "e1", "tag": "button", "kind": "control", "label": "Close"},
            {"ref": "e2", "tag": "input", "type": "password", "kind": "field", "label": "Pass",
                "secret": true, "filled": true},
            {"ref": "e3", "tag": "canvas", "kind": "graphic", "label": ""},
            {"ref": "e4", "tag": "div", "role": "tab", "kind": "control", "label": "Tab",
                "checked": true, "expanded": false}
        ]
    }));
    assert_eq!(page.text, "Hello world");
    assert_eq!(page.viewport, Some((800, 600)));
    assert_eq!(page.scroll_y, Some(40));
    let refs: Vec<_> = page.controls.iter().map(|c| c.reference.as_str()).collect();
    assert_eq!(refs, ["e1", "e2", "e4"], "a canvas is not a control");
    assert_eq!(page.controls[1].filled, Some(true));
    assert_eq!(page.controls[2].role, "tab");
    assert_eq!(page.controls[2].expanded, Some(false));
}

#[test]
fn the_same_page_through_either_browser_says_the_same_sentence() {
    let look = |count: &str, agreed: bool| {
        Observed::from_look(&json!({
            "url": "https://a.test/", "title": "A", "text": format!("Cart\nItems: {count}"),
            "elements": [
                {"ref": "e1", "tag": "button", "kind": "control", "label": "Add"},
                {"ref": "e2", "tag": "input", "type": "checkbox", "kind": "field",
                    "label": "Agree", "checked": agreed}
            ]
        }))
    };
    let snap = |count: &str, agreed: bool| {
        Observed::from_snapshot(&snapshot(
            "https://a.test/",
            &format!("Cart Items: {count}"),
            vec![
                button("c1", "Add"),
                json!({"ref": "c2", "tag": "input", "role": "checkbox", "type": "checkbox",
                    "ariaName": "Agree", "checked": agreed}),
            ],
        ))
    };
    for (from, to, said) in [
        (("0", false), ("0", false), "No visible change."),
        (("0", false), ("1", false), "Page text changed."),
        (("1", false), ("1", true), "1 control changed."),
        (
            ("0", false),
            ("1", true),
            "1 control changed; page text changed.",
        ),
    ] {
        assert_eq!(
            compare(Some(&look(from.0, from.1)), &look(to.0, to.1)).sentence,
            said,
            "look {from:?} -> {to:?}"
        );
        assert_eq!(
            compare(Some(&snap(from.0, from.1)), &snap(to.0, to.1)).sentence,
            said,
            "snapshot {from:?} -> {to:?}"
        );
    }
}

#[test]
fn where_a_control_is_on_the_screen_is_not_a_change() {
    let mut moved = snapshot("https://a.test/", "T", vec![button("c1", "Go")]);
    let before = Observed::from_snapshot(&moved);
    moved["controls"][0]["box"] = json!({"x": 99, "y": 99, "width": 9, "height": 9});
    moved["controls"][0]["selector"] = json!("div > button:nth-of-type(2)");
    let after = Observed::from_snapshot(&moved);
    let result = compare(Some(&before), &after);
    assert_eq!(result.sentence, "No visible change.");
    assert!(!result.changed && result.compared);
}

#[test]
fn a_new_address_wins_over_everything_else() {
    let before =
        Observed::from_snapshot(&snapshot("https://a.test/x", "A", vec![button("c1", "Go")]));
    let mut other = snapshot("https://a.test/y", "B", vec![]);
    other["title"] = json!("Next page");
    let result = compare(Some(&before), &Observed::from_snapshot(&other));
    assert_eq!(
        result.sentence,
        "URL https://a.test/x -> https://a.test/y. Title now \"Next page\"."
    );
    assert!(result.url_changed && result.changed);
    assert_eq!(
        result.controls_changed, 0,
        "a new page's controls are not counted"
    );
    // A change of the hash alone is a change of address.
    let hash = Observed::from_snapshot(&snapshot(
        "https://a.test/x#two",
        "A",
        vec![button("c1", "Go")],
    ));
    assert_eq!(
        compare(Some(&before), &hash).sentence,
        "URL https://a.test/x -> https://a.test/x#two."
    );
}

#[test]
fn removed_controls_count_and_a_scroll_is_a_change() {
    let mut scrolled = snapshot("https://a.test/", "T", vec![button("c1", "A")]);
    let before = Observed::from_snapshot(&snapshot(
        "https://a.test/",
        "T",
        vec![button("c1", "A"), button("c2", "B"), button("c3", "C")],
    ));
    let result = compare(Some(&before), &Observed::from_snapshot(&scrolled));
    assert_eq!(result.sentence, "2 controls changed.");
    scrolled["viewport"]["scrollY"] = json!(600);
    scrolled["controls"] = json!([button("c1", "A"), button("c2", "B"), button("c3", "C")]);
    let result = compare(Some(&before), &Observed::from_snapshot(&scrolled));
    assert_eq!(result.sentence, "Scrolled to y=600.");
}

#[test]
fn addresses_that_differ_only_far_along_are_shown_by_their_ends() {
    let long = format!("https://a.test/{}", "p/".repeat(80));
    let (from, to) = address_pair(&format!("{long}one"), &format!("{long}two"));
    assert!(from.ends_with("one") && to.ends_with("two"), "{from} {to}");
    assert!(from.starts_with('…') && to.starts_with('…'));
    assert!(from.chars().count() <= URL_CHARS + 1);
}

#[test]
fn an_event_beside_the_page_is_not_reported_as_nothing() {
    let page = Observed::from_snapshot(&snapshot("https://a.test/", "T", vec![]));
    let mut same = compare(Some(&page), &page);
    same.also("a dialog was answered");
    assert_eq!(
        same.sentence,
        "A dialog was answered; no other visible change."
    );
    let mut changed = compare(
        Some(&page),
        &Observed::from_snapshot(&snapshot("https://a.test/", "U", vec![])),
    );
    changed.also("a dialog was answered");
    assert_eq!(
        changed.sentence,
        "Page text changed; a dialog was answered."
    );
    let mut unknown = compare(None, &page);
    unknown.also("a dialog was answered");
    assert_eq!(
        unknown.sentence,
        "No earlier observation of this tab to compare with. A dialog was answered."
    );
}

#[test]
fn page_words_are_one_clean_line() {
    assert_eq!(
        one_line("a\nb\t\u{7}c\u{202e}d  e\u{200b}f", 100),
        "a b c d e f"
    );
    // The Arabic letter mark is as invisible, and as able to reorder what is
    // next to it, as the left-to-right mark.
    assert_eq!(one_line("a\u{61c}b\u{200e}c", 100), "a b c");
    assert_eq!(one_line(&"é".repeat(10), 4), "éééé…");
}

#[test]
fn the_budget_holds_whatever_the_page_holds() {
    for controls in [0, 1, 60, 160, 400] {
        for words in [0, 10, 2400, 12_000] {
            for lines in [150, 40, 14] {
                let listed: Vec<Value> = (1..=controls)
                    .map(|n| button(&format!("c{n}"), &format!("Action {n} {}", "y".repeat(70))))
                    .collect();
                let mut snap = snapshot(
                    &format!("https://a.test/{}", "u".repeat(250)),
                    &"word ".repeat(words),
                    listed,
                );
                // Forms and frames take their share of the lines too.
                snap["forms"] = json!(
                    (1..=12)
                        .map(|n| json!({"id": format!("f{n}"), "fields": []}))
                        .collect::<Vec<_>>()
                );
                snap["iframes"] = json!(
                    (1..=8)
                        .map(|n| json!({"index": n, "crossOrigin": true}))
                        .collect::<Vec<_>>()
                );
                let page = Observed::from_snapshot(&snap);
                let comparison = compare(None, &page);
                let text = render(
                    &View {
                        note: "UNTRUSTED.",
                        comparison: Some(&comparison),
                        notices: Vec::new(),
                        action: Some("{\"ref\":\"c1\"}".into()),
                        page: &page,
                    },
                    Budget {
                        chars: 6_000,
                        lines,
                    },
                );
                assert!(
                    text.chars().count() <= 6_000 && text.lines().count() <= lines,
                    "{controls} controls, {words} words, {lines} lines: {} chars, {} lines",
                    text.chars().count(),
                    text.lines().count()
                );
                assert!(
                    !text.contains("cut to the output limit"),
                    "the blocks keep to the budget on their own:\n{text}"
                );
                assert_eq!(
                    text.lines()
                        .nth(1)
                        .map(|line| line.starts_with("Outcome: ")),
                    Some(true),
                    "the outcome is the first line after the label:\n{text}"
                );
                let shown = text
                    .lines()
                    .filter(|l| l.contains(" button \"Action"))
                    .count();
                let omitted = text
                    .lines()
                    .find(|l| l.ends_with("more controls not listed"))
                    .map(|l| {
                        l.trim_start_matches("… ")
                            .split(' ')
                            .next()
                            .unwrap()
                            .parse::<usize>()
                            .unwrap()
                    })
                    .unwrap_or(0);
                assert_eq!(
                    shown + omitted,
                    controls,
                    "every control is listed or counted"
                );
                if omitted > 0 && words > 0 {
                    assert!(text.contains("… text cut at 0 chars"), "text goes first");
                }
            }
        }
    }
}

#[test]
fn controls_that_fit_with_room_for_the_text_marker_are_all_listed() {
    let controls: Vec<Value> = (1..=60)
        .map(|n| button(&format!("c{n}"), &format!("Action {n} {}", "y".repeat(60))))
        .collect();
    let view = |text: &str, cap: usize| {
        let page = Observed::from_snapshot(&snapshot("https://a.test/", text, controls.clone()));
        render(
            &View {
                note: "UNTRUSTED.",
                comparison: None,
                notices: Vec::new(),
                action: None,
                page: &page,
            },
            Budget {
                chars: cap,
                lines: 150,
            },
        )
    };
    // The page with no text at all is the room the controls need.
    let needed = view("", 100_000).chars().count();

    // Room for the controls and for the line that says the text was cut: no
    // control is given up to a text that would not have been shown anyway.
    let text = view(&"word ".repeat(2400), needed + 100);

    assert!(
        text.chars().count() <= needed + 100,
        "{}",
        text.chars().count()
    );
    assert!(
        text.contains("c60 button") && !text.contains("more controls not listed"),
        "every control is listed:\n{text}"
    );
    assert!(
        text.contains("… text cut at "),
        "the text says it was cut:\n{text}"
    );
}

fn only(include: &[&str], page: &Value) -> Observed {
    let include: Vec<Value> = include.iter().map(|name| json!(name)).collect();
    Observed::from_snapshot(page).limited_to(&include)
}

#[test]
fn a_section_that_was_not_read_is_not_compared() {
    let whole = snapshot("https://a.test/", "Hello", vec![button("c1", "Pay")]);
    let mut text_only = whole.clone();
    text_only["controls"] = json!([]);
    let mut controls_only = whole.clone();
    controls_only["text"] = json!("");
    let full = Observed::from_snapshot(&whole);

    // The controls of the later page are not "new" because the earlier page
    // never listed any.
    let after_text = compare(Some(&only(&["text"], &text_only)), &full);
    assert!(after_text.added.is_empty() && after_text.controls_changed == 0);
    assert_eq!(
        after_text.sentence,
        "No visible change in what was compared; controls not read both times."
    );
    assert!(after_text.compared && !after_text.changed);

    // Nor is the text "changed" because the earlier page did not carry it.
    let after_controls = compare(Some(&only(&["controls"], &controls_only)), &full);
    assert!(!after_controls.text_changed);
    assert_eq!(
        after_controls.sentence,
        "No visible change in what was compared; text not read both times."
    );

    let neither = compare(Some(&only(&[], &text_only)), &only(&["forms"], &text_only));
    assert_eq!(
        neither.sentence,
        "No visible change in what was compared; controls and text not read both times."
    );

    // What was read on both sides still counts, and the address still wins.
    let mut moved = whole.clone();
    moved["text"] = json!("Changed");
    let changed = compare(
        Some(&only(&["text"], &text_only)),
        &Observed::from_snapshot(&moved),
    );
    assert_eq!(changed.sentence, "Page text changed.");
    let elsewhere = snapshot("https://b.test/", "Hello", vec![]);
    let url = compare(Some(&only(&[], &elsewhere)), &full);
    assert_eq!(url.sentence, "URL https://b.test/ -> https://a.test/.");
}

#[test]
fn a_partial_page_is_filled_from_the_same_page_and_no_other() {
    let whole = Observed::from_snapshot(&snapshot(
        "https://a.test/",
        "Hello",
        vec![button("c1", "Pay")],
    ));
    let mut text_only = snapshot("https://a.test/", "Hello again", vec![]);
    text_only["controls"] = json!([]);

    let filled = only(&["text"], &text_only).filled_from(Some(&whole));
    assert!(filled.read.controls && filled.read.text);
    assert_eq!(filled.controls.len(), 1, "the controls last seen stay");
    assert_eq!(filled.text, "Hello again", "what was read is not replaced");

    let mut elsewhere = text_only.clone();
    elsewhere["url"] = json!("https://b.test/");
    let kept = only(&["text"], &elsewhere).filled_from(Some(&whole));
    assert!(
        !kept.read.controls && kept.controls.is_empty(),
        "another page's controls are not this page's"
    );
    let alone = only(&["text"], &text_only).filled_from(None);
    assert!(!alone.read.controls);

    // A page that was read whole is taken as it is.
    let again = Observed::from_snapshot(&text_only).filled_from(Some(&whole));
    assert!(again.controls.is_empty() && again.read.controls);
}
