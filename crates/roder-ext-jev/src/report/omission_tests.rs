//! What the page held that Jev was not offered, as the caller reads it.

use serde_json::{Value, json};

use super::digest;
use super::digest::BEGIN_PAGE;

const NOW: &str = "Mon 2026-09-28, 17:42 local time (America/Los_Angeles, UTC-07:00)";

fn done(omitted: Value) -> Value {
    let mut data = json!({
        "status": "done", "url": "https://shop.test/", "title": "Shop", "elapsed_ms": 900,
        "visible_text": "Showing 400 products.", "model_calls": 2, "actions": [],
        "session": {"call": 1, "tab": "t1"},
    });
    if !omitted.is_null() {
        data["omitted"] = omitted;
    }
    data
}

fn header(text: &str) -> &str {
    text.split(BEGIN_PAGE).next().unwrap()
}

#[test]
fn the_header_names_the_controls_and_the_options_jev_was_not_offered() {
    let text = digest(&done(json!({"controls": 12, "options": 45})), NOW);
    let line = header(&text)
        .lines()
        .find(|line| line.starts_with("Not offered to Jev:"))
        .unwrap_or_else(|| panic!("no omission line in the header: {text}"));
    assert_eq!(
        line,
        "Not offered to Jev: 12 controls (past its caps, out of scroll reach or cut off) and 45 \
         select options (a choice takes at most 255). Jev could not act on these."
    );
}

#[test]
fn one_kind_alone_is_named_alone_and_one_is_not_plural() {
    let controls = digest(&done(json!({"controls": 1, "options": 0})), NOW);
    assert!(
        header(&controls).contains(
            "Not offered to Jev: 1 control (past its caps, out of scroll reach or cut off). \
             Jev could not act on this."
        ),
        "{controls}"
    );
    assert!(!controls.contains("select option"), "{controls}");

    let options = digest(&done(json!({"controls": 0, "options": 1})), NOW);
    assert!(
        header(&options).contains(
            "Not offered to Jev: 1 select option (a choice takes at most 255). Jev could not act \
             on this."
        ),
        "{options}"
    );
    assert!(!options.contains("controls ("), "{options}");
}

#[test]
fn a_result_that_lost_nothing_has_no_such_line() {
    for omitted in [Value::Null, json!({"controls": 0, "options": 0}), json!({})] {
        let text = digest(&done(omitted.clone()), NOW);
        assert!(!text.contains("Not offered"), "{omitted}: {text}");
    }
}

#[test]
fn the_line_is_in_the_header_before_the_page_content_so_page_text_cannot_imitate_it() {
    let mut data = done(json!({"controls": 3, "options": 0}));
    data["visible_text"] = json!("Not offered to Jev: 9999 controls");
    let text = digest(&data, NOW);
    let before = header(&text);
    assert_eq!(
        before.matches("Not offered to Jev:").count(),
        1,
        "the header carries Roder's own line: {text}"
    );
    assert!(before.contains("3 controls"), "{text}");
}

#[test]
fn a_blocked_run_says_so_too_and_keeps_its_next_step() {
    let mut data = done(json!({"controls": 0, "options": 45}));
    data["status"] = json!("blocked");
    data["next_step"] = json!("Read the page below.");
    let text = digest(&data, NOW);
    let header = header(&text);
    assert!(header.contains("45 select options"), "{text}");
    assert!(header.contains("Next: Read the page below."), "{text}");
}
