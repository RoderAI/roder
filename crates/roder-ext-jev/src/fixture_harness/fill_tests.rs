//! The verified fill and the select check on real pages: text replaces what
//! a field held without a select-all accelerator, follows focus to an
//! editor the click opened, is typed once more into an editor that took the
//! first keystroke, and a field or select the page refuses is a recorded
//! outcome, not an error. Native date inputs get their ISO value.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::act_on;
use super::scripted::{FieldValues, PlanDecider, find, pick};
use crate::engine::{JevActOutcome, JevStatus};

#[tokio::test]
async fn a_fill_replaces_content_without_the_select_all_accelerator() {
    let harness = harness_or_skip!();
    let mut page = harness.open("editor.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (outcome, next) = act_on(&mut page, &observation, "fill", "Title", Some("New title"))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    let (outcome, _) = act_on(&mut page, &next, "fill", "Notes", Some("New notes"))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    assert_eq!(
        page.evaluate(
            "[document.getElementById('title').value, \
             document.getElementById('notes').innerText, window.blocked]"
        )
        .await
        .unwrap(),
        json!(["New title", "New notes", 0])
    );
}

#[tokio::test]
async fn a_fill_follows_focus_to_the_editor_the_click_opened() {
    let harness = harness_or_skip!();
    let mut page = harness.open("handoff.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (outcome, next) = act_on(&mut page, &observation, "fill", "From", Some("Geneva"))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    assert_eq!(
        page.evaluate(
            "[document.getElementById('from').value, \
             document.getElementById('from-editor').value]"
        )
        .await
        .unwrap(),
        json!(["", "Geneva"])
    );
    // The editor is what the page now offers, holding the text.
    let editor = find(&next, "fill", "From station").unwrap();
    assert_eq!(editor["value"], "Geneva");
    assert!(find(&next, "fill", "From").is_none(), "{next:#}");
}

#[tokio::test]
async fn an_editor_that_took_the_first_keystroke_is_typed_into_again() {
    let harness = harness_or_skip!();
    let mut page = harness.open("handoff.html?on_type=1").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (outcome, _) = act_on(&mut page, &observation, "fill", "From", Some("Geneva"))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    assert_eq!(
        page.evaluate("document.getElementById('from-editor').value")
            .await
            .unwrap(),
        json!("Geneva")
    );
}

#[tokio::test]
async fn a_field_that_drops_the_text_is_recorded_and_the_run_goes_on() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![
        pick("fill", "Promo code"),
        pick("fill", "From"),
        pick("click", "Search"),
    ]));
    let text = Arc::new(FieldValues::new(&[
        ("Promo code", "SPRING"),
        ("From", "Geneva"),
    ]));
    let result = harness
        .run(
            "handoff.html?refuse=1",
            decider,
            Some(text),
            Duration::from_secs(30),
        )
        .await;

    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.stopped_because, None);
    let refusals = result
        .actions
        .iter()
        .map(|action| action.refused.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        refusals,
        vec![
            Some("The field shows \"\", not the typed text."),
            None,
            None
        ]
    );
    let posts = harness.site.wait_for_posts(1, Duration::from_secs(5)).await;
    assert_eq!(posts.len(), 1, "{posts:?}");
    assert_eq!(posts[0].field("from").as_deref(), Some("Geneva"));
    assert_eq!(posts[0].field("promo").as_deref(), Some(""));
}

#[tokio::test]
async fn a_native_date_input_takes_its_iso_value() {
    let harness = harness_or_skip!();
    let mut page = harness.open("booking.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let depart = find(&observation, "fill", "Departure date").unwrap();
    assert_eq!(depart["input_type"], "date");
    assert_eq!(depart["role"], "textbox");

    let (outcome, next) = act_on(
        &mut page,
        &observation,
        "fill",
        "Departure date",
        Some("2026-10-12"),
    )
    .await
    .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    assert_eq!(
        page.evaluate("[depart.value, window.events]")
            .await
            .unwrap(),
        json!(["2026-10-12", ["input", "change"]])
    );
    assert!(
        next["text"]
            .as_str()
            .unwrap()
            .contains("Departing 2026-10-12")
    );

    // A value in any other shape is refused, and says what the field takes.
    let (outcome, _) = act_on(
        &mut page,
        &next,
        "fill",
        "Departure date",
        Some("12/10/2026"),
    )
    .await
    .unwrap();
    assert_eq!(
        outcome.refused.as_deref(),
        Some(
            "The date field kept \"\", not \"12/10/2026\"; it takes an ISO value \
             (2026-09-25 for a date)."
        )
    );
}

#[tokio::test]
async fn a_select_the_page_puts_back_is_refused() {
    let harness = harness_or_skip!();
    let mut page = harness.open("select-refuse.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let (outcome, next) = act_on(&mut page, &observation, "select", "Size → Large", None)
        .await
        .unwrap();
    assert_eq!(
        outcome.refused.as_deref(),
        Some("The page kept \"Medium\" instead of \"Large\".")
    );
    assert!(
        next["text"]
            .as_str()
            .unwrap()
            .contains("Large is out of stock")
    );
    let (outcome, _) = act_on(&mut page, &next, "select", "Size → Small", None)
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
}

#[tokio::test]
async fn a_multi_line_value_lands_as_each_field_keeps_it() {
    let harness = harness_or_skip!();
    let mut page = harness.open("fields.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // Typed into a single-line input a line break becomes a space, an email
    // input also trims the ends, and a textarea keeps it: all three landed.
    let (outcome, next) = act_on(&mut page, &observation, "fill", "Genre", Some("Film\nnoir"))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    let (outcome, next) = act_on(
        &mut page,
        &next,
        "fill",
        "Contact email",
        Some(" ada@example.test\n"),
    )
    .await
    .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    let (outcome, _) = act_on(&mut page, &next, "fill", "Note", Some("Line one\nLine two"))
        .await
        .unwrap();
    assert_eq!(outcome, JevActOutcome::done());
    assert_eq!(
        page.evaluate("[genre.value, email.value, note.value]")
            .await
            .unwrap(),
        json!(["Film noir", "ada@example.test", "Line one\nLine two"])
    );
}

/// A fill whose press navigates has run: the press was sent. It is a step
/// that typed nothing, in the history (and so the budgets and the stall
/// rule), not a stale decision dropped from them.
#[tokio::test]
async fn a_fill_whose_press_navigates_is_a_recorded_step() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![pick("fill", "Search")]));
    let values = Arc::new(FieldValues::new(&[("Search", "lamps")]));
    let result = harness
        .run(
            "fill-navigates.html",
            decider,
            Some(values),
            Duration::from_secs(20),
        )
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.actions.len(), 1, "{result:#?}");
    let step = &result.actions[0];
    assert_eq!(step.kind, "fill");
    assert!(
        step.refused
            .as_deref()
            .is_some_and(|reason| reason.contains("nothing was typed")),
        "{step:#?}"
    );
    assert_eq!(step.page_changed, Some(true));
    assert!(result.url.ends_with("/pages/basic.html"), "{}", result.url);
}
