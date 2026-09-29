//! The result a fallback leaves: who did what at what cost, the hand-over,
//! and the digest the caller reads.

use std::time::Duration;

use serde_json::{Value, json};

use super::run::{FallbackAction, FallbackUsage};
use super::trigger::Trigger;
use super::{FallbackMode, FallbackOutcome, FinalPage, Report, report};
use crate::engine::{JevRunResult, JevStatus, JevStopCause};

fn jev() -> JevRunResult {
    let mut result = JevRunResult::before_start(
        JevStatus::Blocked,
        "https://shop.test/menu",
        Duration::from_millis(3100),
        "x",
    );
    result.stopped_because = None;
    result.stop_cause = Some(JevStopCause::Covered);
    result.model_calls = 5;
    result.actions = vec![crate::engine::JevActionRecord::for_tests("click", "7:00 PM"); 3];
    result
}

fn outcome(status: JevStatus) -> FallbackOutcome {
    FallbackOutcome {
        status,
        stopped_because: (status != JevStatus::Done).then(|| "the page asks to log in".into()),
        message: "7:00 PM at Luna Cafe is selected".into(),
        model: "codex/gpt-6-sol (low)".into(),
        actions: vec![FallbackAction {
            step: 1,
            tool: "key".into(),
            args: json!({"key": "Escape"}),
            target: None,
            result: "Pressed Escape in the page.".into(),
            error: false,
            url: Some("https://shop.test/menu".into()),
            elapsed_ms: 400,
        }],
        model_calls: 2,
        usage: FallbackUsage {
            calls: 2,
            input_tokens: 5000,
            output_tokens: 80,
            incomplete: false,
        },
        elapsed_ms: 9000,
        opened_tabs: Vec::new(),
        target_id: "T".into(),
        last_page: Some(
            json!({"url": "https://shop.test/menu#slot", "title": "Menu", "text": "Selected"}),
        ),
    }
}

fn data() -> Value {
    let mut data = serde_json::to_value(jev()).unwrap();
    data["session"] = json!({"call": 1, "tab": "t1", "tab_note": "new", "tabs": [{"id": "t1"}],
        "max_tabs": 3, "totals": {"calls": 1}});
    data
}

#[test]
fn a_fallback_that_ran_is_the_calls_end_state_with_each_drivers_cost() {
    let mut data = data();
    let page = FinalPage {
        url: "https://shop.test/menu#slot".into(),
        title: "Menu".into(),
        visible_text: "Selected 7:00 PM at Luna Cafe".into(),
        controls: json!([{"label": "Log in", "kind": "click"}]),
        page: json!({"headings": ["Menu"]}),
        observed_elements: 9,
    };
    let ran = Report::Ran {
        trigger: Trigger::Covered,
        outcome: Box::new(outcome(JevStatus::Done)),
    };
    report(
        &mut data,
        &jev(),
        &ran,
        FallbackMode::Auto,
        Some("t1"),
        Some(page),
    );
    assert_eq!(data["status"], "done");
    assert_eq!(data["jev_status"], "blocked");
    assert_eq!(data["stopped_because"], Value::Null);
    assert_eq!(data["elapsed_ms"], 3100 + 9000);
    assert_eq!(data["visible_text"], "Selected 7:00 PM at Luna Cafe");
    assert_eq!(data["observed_elements"], 9);
    assert_eq!(data["drivers"][0]["driver"], "jev");
    assert_eq!(data["drivers"][0]["elapsed_ms"], 3100);
    assert_eq!(data["drivers"][0]["decisions"], 5);
    assert_eq!(data["drivers"][1]["driver"], "fallback");
    assert_eq!(data["drivers"][1]["elapsed_ms"], 9000);
    assert_eq!(data["drivers"][1]["usage"]["input_tokens"], 5000);
    assert_eq!(data["fallback"]["trigger"]["kind"], "covered");
    assert_eq!(data["fallback"]["tab"], "t1");
    // Done: nothing to hand over.
    assert!(data["fallback"].get("tools").is_none());
    let text = crate::report::tool_text(&mut data);
    assert!(text.starts_with("Jev: done, after a fallback."), "{text}");
    assert!(text.contains("Jev itself stopped blocked"), "{text}");
    assert!(
        text.contains("Drivers: Jev 3 actions, 5 decisions, 3.1 s; fallback (codex/gpt-6-sol (low)) 1 tool call, 2 model calls, 9.0 s, 5000 input and 80 output tokens."),
        "{text}"
    );
    assert!(text.contains("What the fallback did:"), "{text}");
    assert!(
        text.contains("The fallback's last word: 7:00 PM at Luna Cafe"),
        "{text}"
    );
    assert!(!text.contains("jev_tab_look"), "{text}");
}

#[test]
fn a_fallback_that_stopped_too_hands_over_and_says_why() {
    let mut data = data();
    let ran = Report::Ran {
        trigger: Trigger::Covered,
        outcome: Box::new(outcome(JevStatus::Blocked)),
    };
    report(
        &mut data,
        &jev(),
        &ran,
        FallbackMode::Auto,
        Some("t1"),
        None,
    );
    assert_eq!(data["status"], "blocked");
    assert_eq!(data["stopped_because"], "the page asks to log in");
    // Without Jev's read the fallback's last look stands in.
    assert_eq!(data["url"], "https://shop.test/menu#slot");
    let text = crate::report::tool_text(&mut data);
    assert!(
        text.contains("Why the fallback stopped: the page asks to log in"),
        "{text}"
    );
    assert!(text.contains("the fallback stopped too"), "{text}");
    assert!(text.contains("jev_tab_look"), "{text}");
}

#[test]
fn a_handover_names_the_tools_the_tab_and_why_none_ran() {
    let mut data = data();
    let handed = Report::Handover {
        trigger: Trigger::NothingToActOn,
        why: Some("claude-code/sonnet runs its own agent".into()),
    };
    report(
        &mut data,
        &jev(),
        &handed,
        FallbackMode::Auto,
        Some("t1"),
        None,
    );
    assert_eq!(data["status"], "blocked");
    assert!(data.get("jev_status").is_none());
    assert_eq!(data["fallback"]["ran"], false);
    assert_eq!(data["fallback"]["tools"].as_array().unwrap().len(), 11);
    let text = crate::report::tool_text(&mut data);
    for said in [
        "Next: Jev could not progress (the page offered nothing Jev can act on",
        "Roder's full browser tools work on this same tab (this thread's Jev tab, t1",
        "jev_tab_click, jev_tab_hover, jev_tab_drag",
        "instead of calling jev_browse again with the same goal or giving up",
        "(The automatic fallback did not run: claude-code/sonnet runs its own agent.)",
    ] {
        assert!(text.contains(said), "{said}: {text}");
    }
    let before = serde_json::to_value(jev()).unwrap();
    let mut untouched = before.clone();
    report(
        &mut untouched,
        &jev(),
        &Report::None,
        FallbackMode::Off,
        None,
        None,
    );
    assert_eq!(untouched, before);
}

#[test]
fn older_page_reads_and_pictures_leave_the_transcript() {
    use roder_api::transcript::{ToolResultRecord, TranscriptItem, VIEW_IMAGE_DISPLAY_KEY};
    let read = |n: usize| {
        TranscriptItem::ToolResult(ToolResultRecord {
            id: format!("r{n}"),
            name: None,
            result: format!("Clicked {n}.\nPage: long read {n}"),
            display_payload: None,
            is_error: false,
        })
    };
    let picture = |n: usize| {
        TranscriptItem::ToolResult(ToolResultRecord {
            id: format!("p{n}"),
            name: None,
            result: format!("Screenshot {n}"),
            display_payload: Some(json!({VIEW_IMAGE_DISPLAY_KEY: {"image_url": "data:x"}})),
            is_error: false,
        })
    };
    let mut transcript = vec![picture(1), read(1), read(2), picture(2), read(3), read(4)];
    super::run::compact(&mut transcript);
    let results = transcript
        .iter()
        .map(|item| match item {
            TranscriptItem::ToolResult(result) => (
                result.result.clone(),
                result
                    .display_payload
                    .as_ref()
                    .is_some_and(|payload| payload.get(VIEW_IMAGE_DISPLAY_KEY).is_some()),
            ),
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        results[0],
        ("Screenshot 1 [picture no longer shown]".into(), false)
    );
    assert!(
        results[1]
            .0
            .ends_with("[page read elided; look again for the current page]")
    );
    assert!(
        results[2]
            .0
            .ends_with("[page read elided; look again for the current page]")
    );
    assert_eq!(results[3], ("Screenshot 2".into(), true));
    assert_eq!(results[4].0, "Clicked 3.\nPage: long read 3");
    assert_eq!(results[5].0, "Clicked 4.\nPage: long read 4");
}
