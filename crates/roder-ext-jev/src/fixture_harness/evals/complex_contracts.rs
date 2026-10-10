//! Compound tasks are split before model tuning. These tests exercise only
//! scripted browser actions; holdout model results belong in a frozen eval.
//! The October 8 holdout was expanded from four to six tasks before any
//! model evaluation: the original four remained byte-equivalent JSON objects,
//! and two longer workflows were added for transfer testing. Freeze hashes
//! must cover the expanded JSON and all three holdout HTML pages.
use std::sync::Arc;

use super::{StepDecider, Task, TaskValues, run_task, validate};

pub(super) fn load(holdout: bool) -> Vec<Task> {
    let raw = if holdout {
        include_str!("../../../tests/fixtures/evals/complex-browser-holdout.json")
    } else {
        include_str!("../../../tests/fixtures/evals/complex-browser-dev.json")
    };
    serde_json::from_str(raw).expect("compound corpus must parse")
}

#[test]
fn complex_contracts_are_well_formed_and_grade_side_effects() {
    for (holdout, expected) in [(false, 8), (true, 6)] {
        let tasks = load(holdout);
        assert_eq!(tasks.len(), expected);
        validate(&tasks).unwrap();
        for task in tasks {
            assert!(!task.expect.text_contains.is_empty());
            assert!(task.expect.dom.contains_key("window.result"));
            assert_eq!(task.expect.dom["window.audit.forbidden"], 0);
            assert!(task.expect.dom["window.audit.writes"].as_u64().unwrap() > 0);
        }
    }
}

async fn contracts(holdout: bool) {
    let base = harness_or_skip!();
    let tasks = load(holdout);
    let mut failures = Vec::new();
    for task in &tasks {
        let harness = base.with_new_site().await;
        let mut outcome = run_task(
            &harness,
            task,
            Arc::new(StepDecider::new(&task.script.plan)),
            Arc::new(TaskValues::new(&task.values)),
            task.expect.probes().cloned().collect(),
            None,
        )
        .await
        .unwrap();
        let errors = task.expect.grade(&outcome);
        if !errors.passed() {
            failures.push(format!("{}: {}", task.id, errors.failures().join("; ")));
            continue;
        }
        // A successful trace cannot hide a forbidden mutation, duplicate
        // write, missing final state, or absent completion evidence.
        for (probe, wrong) in [
            ("window.audit.forbidden", serde_json::json!(1)),
            ("window.audit.writes", serde_json::json!(99)),
            ("window.result", serde_json::Value::Null),
        ] {
            let previous = outcome.probed.dom.insert(probe.into(), Ok(wrong));
            assert!(
                !task.expect.grade(&outcome).passed(),
                "{}: {probe}",
                task.id
            );
            outcome.probed.dom.insert(probe.into(), previous.unwrap());
        }
        outcome.result.visible_text.clear();
        assert!(
            !task.expect.grade(&outcome).passed(),
            "{}: missing completion",
            task.id
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn complex_contracts_dev_pass_real_chrome() {
    contracts(false).await;
}

#[tokio::test]
async fn complex_contracts_holdout_pass_real_chrome() {
    contracts(true).await;
}

#[tokio::test]
async fn complex_contracts_reject_actual_wrong_row_duplicate_and_partial_actions() {
    let base = harness_or_skip!();
    let tasks = load(false);
    let cases = [
        (
            "complex_invoice",
            serde_json::json!([
                {"click":"Send reminder", "context":"INV-481"},
                {"click":"Send reminder", "context":"INV-482"}
            ]),
            "window.audit.forbidden",
            1,
        ),
        (
            "complex_invoice",
            serde_json::json!([
                {"click":"Send reminder", "context":"INV-482"},
                {"click":"Send reminder", "context":"INV-482"}
            ]),
            "window.audit.writes",
            2,
        ),
        (
            "complex_profile",
            serde_json::json!([
                {"fill":"Contact email"},
                {"click":"Review changes"},
                {"done":true}
            ]),
            "window.audit.writes",
            0,
        ),
    ];
    for (id, plan, counter, count) in cases {
        let task = tasks.iter().find(|task| task.id == id).unwrap();
        let plan: Vec<super::Step> = serde_json::from_value(plan).unwrap();
        let harness = base.with_new_site().await;
        let outcome = run_task(
            &harness,
            task,
            Arc::new(StepDecider::new(&plan)),
            Arc::new(TaskValues::new(&task.values)),
            task.expect.probes().cloned().collect(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome.probed.dom[counter].as_ref().unwrap(),
            &serde_json::json!(count),
            "{id}: {counter}"
        );
        assert!(!task.expect.grade(&outcome).passed(), "{id}: {counter}");
    }
}
