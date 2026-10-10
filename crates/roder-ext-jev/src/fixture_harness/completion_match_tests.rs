//! How a `success_condition` is matched against the page a call ended on:
//! case, whitespace (line breaks between nodes, no-break spaces) and
//! zero-width characters are ignored on both sides of a text predicate, a
//! `url_contains` keeps the case of the path and the query, `text_absent`
//! needs the text gone, and what a field holds is not page text.
use super::act_on;
use super::scripted::{FieldValues, PlanDecider, pick};
use super::sessions::{call, call_with, test_sessions};
use crate::fallback::FinalPage;
use crate::session::completion::Completion;
use serde_json::{Value, json};
use std::sync::Arc;

fn passed(result: &Value) -> bool {
    result["status"] == "done" && result["completion_verification"]["status"] == "passed"
}

fn rejected(result: &Value) -> bool {
    result["status"] == "blocked"
        && result["completion_verification"]["status"] == "failed"
        && result["stop_cause"] == "outcome_mismatch"
}

#[tokio::test]
async fn text_contains_matches_across_nodes_whatever_the_case() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let args = |count: &str| {
        json!({"goal":"Add one", "url":harness.site.url("counter.html"),
        "success_condition":{"text_contains":count}})
    };
    // "Count:" and "1" are separate text nodes, joined by a line break.
    let early = call(
        &harness,
        &sessions,
        "fold-early",
        args("count: 1"),
        Arc::new(PlanDecider::new(vec![])),
    )
    .await
    .unwrap();
    assert!(rejected(&early), "{early:#}");
    let correct = call(
        &harness,
        &sessions,
        "fold-correct",
        args("count: 1"),
        Arc::new(PlanDecider::new(vec![pick("click", "Add one")])),
    )
    .await
    .unwrap();
    assert!(passed(&correct), "{correct:#}");
    // The result still shows the page as it is, line break and all.
    assert!(
        correct["visible_text"]
            .as_str()
            .unwrap()
            .contains("Count:\n1"),
        "{correct:#}"
    );
}

#[tokio::test]
async fn a_condition_true_at_load_reports_as_it_always_did() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    for (thread, wanted) in [("load-exact", "Count:\n0"), ("load-folded", "COUNT: 0")] {
        let result = call(
            &harness,
            &sessions,
            thread,
            json!({"goal":"Do nothing", "url":harness.site.url("counter.html"),
            "success_condition":{"text_contains":wanted}}),
            Arc::new(PlanDecider::new(vec![])),
        )
        .await
        .unwrap();
        assert!(passed(&result), "{wanted:?}: {result:#}");
        assert_eq!(result["actions"].as_array().map(Vec::len), Some(0));
    }
}

#[tokio::test]
async fn no_break_and_zero_width_characters_do_not_hide_a_match() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    // The page says "ORDER", two no-break spaces, "Con", a zero-width
    // space and "firmed".
    let result = call(
        &harness,
        &sessions,
        "fold-nbsp",
        json!({"goal":"Finish the order", "url":harness.site.url("completion-state.html"),
        "success_condition":{"url_contains":"/pages/completion-state.html",
            "text_contains":"order confirmed"}}),
        Arc::new(PlanDecider::new(vec![pick("click", "Finish order")])),
    )
    .await
    .unwrap();
    assert!(passed(&result), "{result:#}");
    // The same words with the page's own odd characters in the predicate.
    let result = call(
        &harness,
        &sessions,
        "fold-nbsp-both",
        json!({"goal":"Finish the order", "url":harness.site.url("completion-state.html"),
        "success_condition":{"text_contains":"order\u{a0}\u{a0} con\u{200b}firmed\n"}}),
        Arc::new(PlanDecider::new(vec![pick("click", "Finish order")])),
    )
    .await
    .unwrap();
    assert!(passed(&result), "{result:#}");
}

#[tokio::test]
async fn url_contains_needs_the_pages_own_case_in_its_path() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let url = harness.site.url("completion-state.html");
    // The page text says what is wanted: only the address differs, in case.
    let result = call(
        &harness,
        &sessions,
        "url-case",
        json!({"goal":"Finish the order", "url":url,
        "success_condition":{"url_contains":"/pages/COMPLETION-state.HTML",
            "text_contains":"order confirmed"}}),
        Arc::new(PlanDecider::new(vec![pick("click", "Finish order")])),
    )
    .await
    .unwrap();
    assert!(rejected(&result), "{result:#}");
    assert_eq!(
        result["completion_verification"]["unmet"],
        json!(["url_contains"])
    );
    // The scheme of a full address is free, its path is not.
    let scheme = format!("HTTP{}", &url["http".len()..]);
    let result = call(
        &harness,
        &sessions,
        "url-scheme-case",
        json!({"goal":"Finish the order", "url":url,
        "success_condition":{"url_contains":scheme}}),
        Arc::new(PlanDecider::new(vec![])),
    )
    .await
    .unwrap();
    assert!(passed(&result), "{result:#}");
}

#[tokio::test]
async fn text_absent_needs_the_text_gone() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let args = |absent: &str| {
        json!({"goal":"Finish the order", "url":harness.site.url("completion-state.html"),
        "success_condition":{"text_absent":absent}})
    };
    // The progress line still shows, with a no-break space in it.
    let early = call(
        &harness,
        &sessions,
        "absent-early",
        args("saving order…"),
        Arc::new(PlanDecider::new(vec![])),
    )
    .await
    .unwrap();
    assert!(rejected(&early), "{early:#}");
    assert_eq!(
        early["completion_verification"]["text_absent"],
        "saving order…"
    );
    let finished = call(
        &harness,
        &sessions,
        "absent-finished",
        args("saving order…"),
        Arc::new(PlanDecider::new(vec![pick("click", "Finish order")])),
    )
    .await
    .unwrap();
    assert!(passed(&finished), "{finished:#}");
    // The check names what it is and what it is not.
    let mut data = finished.clone();
    let text = crate::report::tool_text(&mut data);
    assert!(text.contains("Completion check: condition met"), "{text}");
    let mut data = early.clone();
    let text = crate::report::tool_text(&mut data);
    assert!(
        text.contains("Completion check: condition not met"),
        "{text}"
    );
    assert!(text.contains("text_absent"), "{text}");
    assert!(
        !text.to_ascii_lowercase().contains("verified the goal"),
        "{text}"
    );
}

#[tokio::test]
async fn all_predicates_must_hold_and_the_failing_one_is_named() {
    let harness = harness_or_skip!();
    let result = call(
        &harness,
        &test_sessions(),
        "named",
        json!({"goal":"Finish the order", "url":harness.site.url("completion-state.html"),
        "success_condition":{"text_contains":"order confirmed", "text_absent":"saving order…"}}),
        Arc::new(PlanDecider::new(vec![pick("click", "Finish order")])),
    )
    .await
    .unwrap();
    assert!(passed(&result), "{result:#}");
    assert_eq!(result["completion_verification"]["unmet"], json!([]));
    let result = call(
        &harness,
        &test_sessions(),
        "named-failing",
        json!({"goal":"Finish the order", "url":harness.site.url("completion-state.html"),
        "success_condition":{"url_contains":"completion-state", "text_contains":"order confirmed"}}),
        Arc::new(PlanDecider::new(vec![])),
    )
    .await
    .unwrap();
    assert!(rejected(&result), "{result:#}");
    assert_eq!(
        result["completion_verification"]["unmet"],
        json!(["text_contains"])
    );
}

#[tokio::test]
async fn a_typed_value_is_not_page_text() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let typed = || Some(Arc::new(FieldValues::new(&[("Note", "order confirmed")])) as Arc<_>);
    // Typing the phrase into the note field does not make the page say it.
    let echoed = call_with(
        &harness,
        &sessions,
        "echo",
        json!({"goal":"Type the note", "url":harness.site.url("completion-state.html"),
        "success_condition":{"text_contains":"order confirmed"}}),
        Arc::new(PlanDecider::new(vec![pick("fill", "Note")])),
        typed(),
    )
    .await
    .unwrap();
    assert!(rejected(&echoed), "{echoed:#}");
    // The result still shows what the field holds.
    assert!(
        echoed["visible_text"]
            .as_str()
            .unwrap()
            .contains("order confirmed"),
        "{echoed:#}"
    );
    // So the phrase is absent from the page's text, as the page says.
    let absent = call_with(
        &harness,
        &sessions,
        "echo-absent",
        json!({"goal":"Type the note", "url":harness.site.url("completion-state.html"),
        "success_condition":{"text_absent":"order confirmed"}}),
        Arc::new(PlanDecider::new(vec![pick("fill", "Note")])),
        typed(),
    )
    .await
    .unwrap();
    assert!(passed(&absent), "{absent:#}");
    // The page's own text still counts when a field holds the same words.
    let both = call_with(
        &harness,
        &sessions,
        "echo-and-page",
        json!({"goal":"Type the note, then finish", "url":harness.site.url("completion-state.html"),
        "success_condition":{"text_contains":"order confirmed"}}),
        Arc::new(PlanDecider::new(vec![
            pick("fill", "Note"),
            pick("click", "Finish order"),
        ])),
        typed(),
    )
    .await
    .unwrap();
    assert!(passed(&both), "{both:#}");
}

/// The page `observation` shows, as the completion check receives it.
fn final_page(observation: &Value) -> FinalPage {
    FinalPage {
        url: observation["url"].as_str().unwrap().into(),
        visible_text: observation["text"].as_str().unwrap().into(),
        typed_values: observation["typed_values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().into())
            .collect(),
        ..FinalPage::default()
    }
}

fn holds(condition: Value, observation: &Value) -> bool {
    Completion::parse(&json!({ "success_condition": condition }))
        .unwrap()
        .unwrap()
        .matches(&final_page(observation))
}

#[tokio::test]
async fn the_observation_lists_the_field_values_its_text_holds() {
    let harness = harness_or_skip!();
    let mut page = harness.open("fields.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // A textarea's passage counts; the password's value is never read.
    assert_eq!(
        observation["typed_values"],
        json!(["Meet at the north gate at nine."])
    );
    let (_, next) = act_on(&mut page, &observation, "fill", "Genre", Some("Drama"))
        .await
        .unwrap();
    assert_eq!(
        next["typed_values"],
        json!(["Drama", "Meet at the north gate at nine."])
    );
    let text = next["text"].as_str().unwrap();
    assert!(
        text.contains("Drama") && text.contains("north gate"),
        "{text}"
    );
    assert!(!next.to_string().contains("hunter2"), "{next:#}");
    // The page's own words are still there for a check; the fields' are not.
    assert!(holds(json!({"text_contains":"movie SEARCH"}), &next));
    assert!(!holds(json!({"text_contains":"drama"}), &next));
    assert!(holds(json!({"text_absent":"drama"}), &next));
    assert!(!holds(json!({"text_contains":"north gate at nine"}), &next));
    // A value of several lines is taken out whole.
    page.evaluate("document.getElementById('reply').value = 'first line\\nsecond line'")
        .await
        .unwrap();
    let many = page.observe().await.unwrap();
    assert_eq!(many["typed_values"][2], "first line\nsecond line");
    assert!(
        many["text"]
            .as_str()
            .unwrap()
            .contains("first line\nsecond line")
    );
    assert!(holds(json!({"text_absent":"second line"}), &many));
    assert!(holds(
        json!({"text_contains":"Waiting for your guess"}),
        &many
    ));
}

#[tokio::test]
async fn a_field_value_the_text_limit_cuts_is_still_not_page_text() {
    let harness = harness_or_skip!();
    let mut page = harness.open("completion-state.html").await.unwrap();
    // 5,501 characters of the page's own words come first, so the 6,000 the
    // page text keeps end inside the note field's 2,000-character value.
    page.evaluate(
        "(() => {
            const filler = document.createElement('p');
            filler.style.cssText = 'font-size:6px;line-height:6px;margin:0';
            filler.textContent = 'filler '.repeat(786).trim();
            document.body.insertBefore(filler, document.body.firstChild);
            document.getElementById('note').value = 'order confirmed ' + 'x'.repeat(1984);
        })()",
    )
    .await
    .unwrap();
    let observation = page.observe().await.unwrap();
    let text = observation["text"].as_str().unwrap();
    // The limit fell inside the value: the line it left is a prefix of it.
    let kept = text.rsplit('\n').next().unwrap();
    assert!(
        text.encode_utf16().count() > 5990 && kept.starts_with("order confirmed x"),
        "{} units, last line {:?}",
        text.encode_utf16().count(),
        kept.chars().take(40).collect::<String>()
    );
    assert!(kept.len() < 2000, "{}", kept.len());
    // The prefix is reported as the field's, so the check does not read it
    // as the page saying the phrase.
    assert_eq!(
        observation["typed_values"],
        json!([kept]),
        "{observation:#}"
    );
    assert!(!holds(
        json!({"text_contains":"order confirmed"}),
        &observation
    ));
    assert!(holds(
        json!({"text_absent":"order confirmed"}),
        &observation
    ));
    // The page's own words before it still count.
    assert!(holds(
        json!({"text_contains":"filler filler"}),
        &observation
    ));
}

#[tokio::test]
async fn an_unfilled_page_lists_no_field_values() {
    let harness = harness_or_skip!();
    let mut page = harness.open("completion-state.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    assert_eq!(observation["typed_values"], json!([]));
}
