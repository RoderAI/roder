//! What a step's effect says on a real page (`effects.html`): the fields a
//! refused submit still needs, an error status a link landed on, and
//! whether a choice clicked set anything (see `effects`).

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::scripted::{FieldValues, Pick, PlanDecider, find, pick};
use crate::engine::{JevRunResult, JevStatus, JevStopCause, JevTextValueResolver};

async fn run(
    harness: &super::Harness,
    plan: Vec<Pick>,
    text: Option<Arc<dyn JevTextValueResolver>>,
) -> JevRunResult {
    harness
        .run(
            "effects.html",
            Arc::new(PlanDecider::new(plan)),
            text,
            Duration::from_secs(30),
        )
        .await
}

fn effects(result: &JevRunResult) -> Vec<String> {
    result
        .actions
        .iter()
        .map(|action| action.effect.clone().unwrap_or_default())
        .collect()
}

/// A submit the form refuses changes nothing; the effect names the field it
/// is waiting for, and one a script marked invalid once it is, each only in
/// the form of the button pressed.
#[tokio::test]
async fn a_refused_submit_names_the_fields_its_form_still_needs() {
    let harness = harness_or_skip!();
    let result = run(
        &harness,
        vec![pick("click", "Search"), pick("click", "Send")],
        None,
    )
    .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(
        effects(&result),
        [
            "nothing visible changed; fields the form still needs: 1 control: Return",
            // Only Send's own form: Return belongs to the trip search.
            "nothing visible changed; fields the form still needs: 1 control: Email",
        ],
        "{result:#?}"
    );
}

/// A link to a page that is not there lands on a 404, and says so.
#[tokio::test]
async fn a_link_that_lands_on_an_error_says_so() {
    let harness = harness_or_skip!();
    let result = run(&harness, vec![pick("click", "Old offers")], None).await;
    let effect = &effects(&result)[0];
    assert!(
        effect.contains("/pages/gone.html; HTTP 404: destination did not load successfully"),
        "{effect}"
    );
}

/// An autocomplete pick that leaves the typed text as it was set the field;
/// a suggestion that only closes the list set nothing, and says so.
#[tokio::test]
async fn a_suggestion_picked_counts_only_when_the_field_shows_it() {
    let harness = harness_or_skip!();
    let text = || -> Option<Arc<dyn JevTextValueResolver>> {
        Some(Arc::new(FieldValues::new(&[("Where from?", "London")])))
    };
    let picked = run(
        &harness,
        vec![
            pick("fill", "Where from?"),
            pick("click", "London, United Kingdom"),
        ],
        text(),
    )
    .await;
    let effect = &effects(&picked)[1];
    assert!(effect.starts_with("removed 3 controls:"), "{effect}");
    assert!(!effect.contains("set no value"), "{effect}");
    let missed = run(
        &harness,
        vec![pick("fill", "Where from?"), pick("click", "Nowhere at all")],
        text(),
    )
    .await;
    let effect = &effects(&missed)[1];
    // Nothing was set, so the field the form still needs is named too.
    assert!(
        effect.ends_with(
            "; Nowhere at all set no value; fields the form still needs: 1 control: Return"
        ),
        "{effect}"
    );
}

/// A choice the page ignores while it redraws something else changes the
/// page every time but sets nothing: three in a row stall the run.
#[tokio::test]
async fn choices_that_set_nothing_stall_the_run() {
    let harness = harness_or_skip!();
    let result = run(
        &harness,
        vec![
            pick("click", "One way"),
            pick("click", "One way"),
            pick("click", "One way"),
            pick("click", "One way"),
        ],
        None,
    )
    .await;
    assert!(
        result
            .actions
            .iter()
            .all(|action| action.page_changed == Some(true)),
        "{result:#?}"
    );
    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(
        result.stop_cause,
        Some(JevStopCause::Stalled),
        "{result:#?}"
    );
    assert_eq!(result.actions.len(), 3, "{result:#?}");
}

/// The snapshot marks a field its form will not submit without: required and
/// empty, a required radio group with none checked (by the group's
/// validity, or by its radios for `aria-required`), a required checkbox
/// unchecked, or one marked invalid.
#[tokio::test]
async fn the_snapshot_marks_the_fields_a_form_still_needs() {
    let harness = harness_or_skip!();
    let mut page = harness.open("basic.html").await.unwrap();
    let markup = concat!(
        "<form><label>First<input type=\"radio\" name=\"choice\" required checked></label>",
        "<label>Second<input type=\"radio\" name=\"choice\" required></label>",
        "<label>Terms<input type=\"checkbox\" required></label>",
        "<label>Yes<input type=\"radio\" name=\"ans\" aria-required=\"true\" checked></label>",
        "<label>No<input type=\"radio\" name=\"ans\" aria-required=\"true\"></label>",
        "<label>Name<input required value=\"Ada\"></label>",
        "<label>Phone<input required value=\"  \"></label>",
        "<label>Code<input type=\"password\" required></label>",
        "<label>Notes<textarea></textarea></label>",
        "<div role=\"textbox\" aria-label=\"Card\" aria-invalid=\"true\" contenteditable=\"true\"></div></form>",
        "<form><label>Other form<input type=\"radio\" name=\"choice\" required></label></form>",
    );
    page.evaluate(&format!(
        "document.body.innerHTML = {}",
        serde_json::to_string(markup).unwrap()
    ))
    .await
    .unwrap();
    let observation = page.observe().await.unwrap();
    let blocking = |observation: &Value, kind: &str, label: &str| {
        find(observation, kind, label)
            .unwrap_or_else(|| panic!("{kind} {label} in {observation:#}"))
            .get("blocking")
            .cloned()
    };
    for (kind, label, marked) in [
        ("click", "First", None),
        ("click", "Second", None),
        ("click", "Terms", Some(json!(true))),
        ("click", "Yes", None),
        ("click", "No", None),
        ("fill", "Name", None),
        ("fill", "Phone", Some(json!(true))),
        ("fill", "Code", Some(json!(true))),
        ("fill", "Notes", None),
        ("fill", "Card", Some(json!(true))),
        ("click", "Other form", Some(json!(true))),
    ] {
        assert_eq!(blocking(&observation, kind, label), marked, "{label}");
    }
    page.evaluate("document.querySelector('input').checked = false")
        .await
        .unwrap();
    let observation = page.observe().await.unwrap();
    assert_eq!(blocking(&observation, "click", "First"), Some(json!(true)));
    assert_eq!(blocking(&observation, "click", "Second"), Some(json!(true)));
    page.close().await.ok();
}
