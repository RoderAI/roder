//! What a row says about a run: the agent's verdict, the truth the page and
//! the POST log show, and a `false_green` where the two part ways.
//!
//! The first group needs no browser: it builds the outcomes by hand. The
//! last runs `search-decoy.html`, a page made after the one false DONE the
//! live corpus recorded (`enter_to_search`: status done, no POST), on real
//! Chrome.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::probe::Probed;
use super::watch::Watched;
use super::{FallbackRow, Outcome, Row, StepDecider, Task, TaskValues, run_task, table};
use crate::engine::{JevRunResult, JevStatus};
use crate::fixture_harness::Harness;

const STATUS_TEXT: &str = "document.querySelector('#status').textContent";

fn task(value: Value) -> Task {
    serde_json::from_value(value).unwrap()
}

/// An outcome that ended `status`, with the DOM reads the probes would have
/// made, and no POST.
fn outcome(status: JevStatus, dom: &[(&str, Value)]) -> Outcome {
    let mut result = JevRunResult::before_start(
        status,
        "http://127.0.0.1/pages/search-decoy.html",
        Duration::ZERO,
        "",
    );
    result.stopped_because = None;
    Outcome {
        result,
        probed: Probed {
            dom: dom
                .iter()
                .map(|(expression, value)| (expression.to_string(), Ok(value.clone())))
                .collect::<BTreeMap<_, _>>(),
        },
        posts: Vec::new(),
        wall_ms: 0,
        watch: Watched::default(),
        after: None,
    }
}

/// The row a tier would write for `outcome`, as it serialises.
fn row_json(task: &Task, outcome: &Outcome) -> Value {
    let graded = task.expect.grade(outcome);
    let row = Row::new(task, "keyless", Vec::new(), outcome, graded);
    serde_json::to_value(row).unwrap()
}

fn status_task(status: &str) -> Task {
    task(json!({
        "id": "status_text",
        "covers": "a status line the page draws",
        "page": "search-decoy.html",
        "goal": "Search the site for trail shoes.",
        "expect": {
            "status": status,
            "stopped": false,
            "dom": {STATUS_TEXT: "Sent: trail shoes"}
        },
        "script": {"plan": [{"done": true}]}
    }))
}

#[test]
fn a_done_with_a_failing_dom_probe_is_a_false_green() {
    let outcome = outcome(JevStatus::Done, &[(STATUS_TEXT, json!(""))]);
    let row = row_json(&status_task("done"), &outcome);
    assert_eq!(row["verdict_ok"], true, "{row}");
    assert_eq!(row["truth_ok"], false, "{row}");
    assert_eq!(row["false_green"], true, "{row}");
    let failures = row["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1, "{row}");
    assert!(failures[0].as_str().unwrap().contains("dom"), "{row}");
}

#[test]
fn a_done_the_page_confirms_is_green_without_being_false() {
    let outcome = outcome(
        JevStatus::Done,
        &[(STATUS_TEXT, json!("Sent: trail shoes"))],
    );
    let row = row_json(&status_task("done"), &outcome);
    assert_eq!(row["verdict_ok"], true, "{row}");
    assert_eq!(row["truth_ok"], true, "{row}");
    assert_eq!(row["false_green"], false, "{row}");
    assert_eq!(row["failures"], json!([]), "{row}");
}

#[test]
fn a_verdict_the_task_did_not_ask_for_is_not_a_false_green() {
    // The task wants an honest BLOCKED; the run says DONE and the page
    // agrees with nothing. The claim missed, so it is a plain failure.
    let outcome = outcome(JevStatus::Done, &[(STATUS_TEXT, json!(""))]);
    let row = row_json(&status_task("blocked"), &outcome);
    assert_eq!(row["verdict_ok"], false, "{row}");
    assert_eq!(row["truth_ok"], false, "{row}");
    assert_eq!(row["false_green"], false, "{row}");
}

#[test]
fn a_claim_that_matches_over_a_page_that_agrees_is_a_pass_for_a_blocked_task() {
    let task = task(json!({
        "id": "honest_block",
        "covers": "a run that rightly gives up",
        "page": "search-decoy.html",
        "goal": "Search the site for trail shoes.",
        "expect": {"status": "blocked", "stopped": true, "posts": []},
        "script": {"plan": [{"blocked": true}]}
    }));
    let mut blocked = outcome(JevStatus::Blocked, &[]);
    blocked.result.stopped_because = Some("nothing serves the goal".into());
    let row = row_json(&task, &blocked);
    assert_eq!(row["verdict_ok"], true, "{row}");
    assert_eq!(row["truth_ok"], true, "{row}");
    assert_eq!(row["false_green"], false, "{row}");
}

#[test]
fn the_row_carries_the_split_and_no_pass_field() {
    let outcome = outcome(
        JevStatus::Done,
        &[(STATUS_TEXT, json!("Sent: trail shoes"))],
    );
    let row = row_json(&status_task("done"), &outcome);
    let keys = row.as_object().unwrap();
    assert!(!keys.contains_key("pass"), "{row}");
    for key in ["verdict_ok", "truth_ok", "false_green"] {
        assert!(keys.contains_key(key), "{key} missing: {row}");
    }
}

#[test]
fn a_task_that_could_not_run_claims_nothing() {
    let task = status_task("done");
    let row = Row::errored(&task, "keyless", &anyhow::anyhow!("Chrome did not start"));
    let row = serde_json::to_value(row).unwrap();
    assert_eq!(row["verdict_ok"], false, "{row}");
    assert_eq!(row["truth_ok"], false, "{row}");
    assert_eq!(row["false_green"], false, "{row}");
}

#[test]
fn the_table_tells_a_false_green_from_a_miss_and_counts_both() {
    let task = status_task("done");
    let page = |text: &str| outcome(JevStatus::Done, &[(STATUS_TEXT, json!(text))]);
    let row = |outcome: &Outcome, repeat: usize| {
        let mut row = Row::new(
            &task,
            "live",
            Vec::new(),
            outcome,
            task.expect.grade(outcome),
        );
        row.repeat = repeat;
        row
    };
    let blocked = {
        let mut blocked = outcome(JevStatus::Blocked, &[]);
        blocked.result.stopped_because = Some("gave up".into());
        blocked
    };
    // Run 2 is first on purpose: the table orders a task's runs itself.
    let rows = [
        row(&page(""), 2),
        row(&page("Sent: trail shoes"), 1),
        row(&blocked, 3),
    ];
    let out = table(std::slice::from_ref(&task), &rows);
    let lines = out.lines().collect::<Vec<_>>();
    assert!(lines[1].contains("pass"), "{out}");
    assert!(lines[2].contains("FALSE-GREEN"), "{out}");
    assert!(
        lines[3].contains("FAIL") && !lines[3].contains("FALSE"),
        "{out}"
    );
    assert_eq!(lines[4], "1/3 passed, 1 false green", "{out}");
}

#[test]
fn a_fallback_that_did_not_run_carries_jevs_marks_and_misses_the_task() {
    let task = status_task("done");
    let outcome = outcome(JevStatus::Done, &[(STATUS_TEXT, json!(""))]);
    let jev = task.expect.grade(&outcome).marks();
    let after = FallbackRow::grade(&task, &outcome, jev);
    assert!(!after.ran);
    assert_eq!(after.marks, jev);
    assert!(after.marks.false_green);
    let missed = after.missed();
    assert_eq!(missed.truth, ["the fallback did not run"]);
    assert!(missed.verdict.is_empty());
}

/// The decoy page, played two ways on the same page: the plan a confused
/// model follows (type, press the button that draws results, say DONE) and
/// the plan that really searches.
fn search_task(id: &str, plan: Value) -> Task {
    task(json!({
        "id": id,
        "covers": "a page that draws a finished search without posting one",
        "page": "search-decoy.html",
        "goal": "Search the site for trail shoes.",
        "values": {"Search": "trail shoes"},
        "expect": {
            "status": "done",
            "stopped": false,
            "url_ends_with": "/submit/search",
            "posts": [{"path": "/submit/search", "fields": {"q": "trail shoes"}}]
        },
        "script": {"plan": plan}
    }))
}

async fn play(base: &Harness, task: &Task) -> (Outcome, Value) {
    let harness = base.with_new_site().await;
    let outcome = run_task(
        &harness,
        task,
        Arc::new(StepDecider::new(&task.script.plan)),
        Arc::new(TaskValues::new(&task.values)),
        task.expect.probes().cloned().collect(),
        None,
    )
    .await
    .unwrap();
    let row = row_json(task, &outcome);
    (outcome, row)
}

#[tokio::test]
async fn the_decoy_page_gives_a_false_done_the_split_catches() {
    let base = harness_or_skip!();
    let decoy = search_task(
        "decoy_search",
        json!([{"fill": "Search"}, {"click": "Preview results"}]),
    );
    let (outcome, row) = play(&base, &decoy).await;
    // What the recorded run looked like: DONE, nothing posted.
    assert_eq!(row["status"], "done", "{row}");
    assert!(outcome.posts.is_empty(), "{:?}", outcome.posts);
    assert!(
        outcome
            .result
            .visible_text
            .contains("Showing results for trail shoes"),
        "the decoy must look finished: {}",
        outcome.result.visible_text
    );
    assert_eq!(row["verdict_ok"], true, "{row}");
    assert_eq!(row["truth_ok"], false, "{row}");
    assert_eq!(row["false_green"], true, "{row}");

    // The same page, searched properly, is green all the way down.
    let honest = search_task(
        "honest_search",
        json!([{"fill": "Search"}, {"enter": "Search"}]),
    );
    let (outcome, row) = play(&base, &honest).await;
    assert_eq!(outcome.posts.len(), 1, "{:?}", outcome.posts);
    assert_eq!(row["verdict_ok"], true, "{row}");
    assert_eq!(row["truth_ok"], true, "{row}");
    assert_eq!(row["false_green"], false, "{row}");
}
