//! The laps and readings a row carries, on real Chrome with the scripted
//! decider: what a 400-card page offered and left out, how each settle
//! ended, and how much of a run's time the laps explain.

use std::sync::Arc;

use serde_json::json;

use super::{Outcome, Row, StepDecider, Task, TaskValues, load_all, run_task};
use crate::fixture_harness::Harness;

async fn play(base: &Harness, task: &Task) -> Outcome {
    let harness = base.with_new_site().await;
    run_task(
        &harness,
        task,
        Arc::new(StepDecider::new(&task.script.plan)),
        Arc::new(TaskValues::new(&task.values)),
        task.expect.probes().cloned().collect(),
        None,
    )
    .await
    .unwrap()
}

fn corpus_task(id: &str) -> Task {
    load_all()
        .unwrap()
        .into_iter()
        .find(|task| task.id == id)
        .unwrap_or_else(|| panic!("no task {id}"))
}

#[tokio::test]
async fn a_run_records_what_a_300_control_page_offered_and_left_out() {
    let base = harness_or_skip!();
    let task: Task = serde_json::from_value(json!({
        "id": "big_store",
        "covers": "a page with far more controls than one look offers",
        "page": "big.html?n=400",
        "goal": "Look at the store.",
        "expect": {"status": "done"},
        "script": {"plan": [{"done": true}]}
    }))
    .unwrap();
    let outcome = play(&base, &task).await;
    let look = &outcome.watch.looks[0];
    assert!(
        (200..=260).contains(&look.offered),
        "the snapshot offers at most 250 controls and its few extras: {look:?}"
    );
    assert!(
        look.omitted_actions >= 100,
        "400 cards leave most controls out: {look:?}"
    );
    // The same facts reach the row a tier writes.
    let row = Row::new(
        &task,
        "keyless",
        Vec::new(),
        &outcome,
        task.expect.grade(&outcome),
    );
    let row = serde_json::to_value(row).unwrap();
    assert_eq!(
        row["watch"]["looks"][0]["omitted_actions"],
        json!(look.omitted_actions),
        "{row}"
    );
}

#[tokio::test]
async fn a_run_records_each_settle_and_every_phase_it_spent_time_in() {
    let base = harness_or_skip!();
    let task = corpus_task("contact_form");
    let outcome = play(&base, &task).await;
    let watch = &outcome.watch;
    let result = &outcome.result;

    // One look at the start, then one per step; the first has no input
    // behind it, every later one waited for the step before it.
    assert_eq!(watch.looks.len(), result.actions.len() + 1, "{watch:?}");
    assert_eq!(watch.looks[0].settle, None, "{watch:?}");
    for look in &watch.looks[1..] {
        let settle = look.settle.as_ref().unwrap_or_else(|| panic!("{watch:?}"));
        assert!(
            ["quiet", "listbox", "cap", "unanswered"].contains(&settle.reason.as_str()),
            "{settle:?}"
        );
    }

    // The loop's calls, counted from outside, match what the result says.
    assert_eq!(watch.laps["decide"].calls, result.model_calls, "{watch:?}");
    assert_eq!(watch.laps["text"].calls, result.text_calls, "{watch:?}");
    assert_eq!(
        watch.laps["settle"].calls,
        result.actions.len(),
        "{watch:?}"
    );
    assert_eq!(watch.laps["snapshot"].calls, watch.looks.len(), "{watch:?}");
    assert!(watch.laps["act"].calls >= result.actions.len(), "{watch:?}");
    assert_eq!(watch.run_ms, result.elapsed_ms);
    eprintln!(
        "contact_form: run {} ms, laps {} ms: {:?}",
        watch.run_ms, watch.attributed_ms, watch.laps
    );
}
