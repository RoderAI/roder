//! Full-goal completion, duplicate mutations and explicit non-submission.
//! Holdout model results are not used until the candidate is frozen.
use super::{Step, StepDecider, Task, TaskValues, run_task, validate};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) fn load(holdout: bool) -> Vec<Task> {
    serde_json::from_str(if holdout {
        include_str!("../../../tests/fixtures/evals/completion-holdout.json")
    } else {
        include_str!("../../../tests/fixtures/evals/completion-dev.json")
    })
    .unwrap()
}

#[tokio::test]
async fn completion_contracts_pass_scripted_browser_actions() {
    let base = harness_or_skip!();
    for holdout in [false, true] {
        let tasks = load(holdout);
        validate(&tasks).unwrap();
        assert_eq!(tasks.len(), 4);
        for task in tasks {
            let harness = base.with_new_site().await;
            let outcome = run_task(
                &harness,
                &task,
                Arc::new(StepDecider::new(&task.script.plan)),
                Arc::new(TaskValues::new(&task.values)),
                task.expect.probes().cloned().collect(),
                None,
            )
            .await
            .unwrap();
            let grade = task.expect.grade(&outcome);
            assert!(grade.passed(), "{}: {:?}", task.id, grade.failures());
        }
    }
}

#[tokio::test]
async fn completion_contracts_reject_partial_duplicate_and_forbidden_work() {
    let base = harness_or_skip!();
    let mut tasks = load(false);
    tasks.extend(load(true));
    for (id, plan, audit, result) in [
        (
            "completion_publish",
            json!([{"fill":"Note title"},{"click":"Preview note"},{"done":true}]),
            json!({"writes":0,"forbidden":0,"previews":1}),
            Value::Null,
        ),
        (
            "completion_already_queued",
            json!([{"click":"Queue another receipt"},{"done":true}]),
            json!({"writes":1,"forbidden":1,"previews":0}),
            json!({"queued":true,"deliveries":2}),
        ),
        (
            "completion_selection_only",
            json!([{"select":"Density → Compact"},{"click":"Apply layout"},{"done":true}]),
            json!({"writes":1,"forbidden":1,"previews":0}),
            json!({"density":"Compact","applied":true}),
        ),
        (
            "completion_holdout_document",
            json!([{"select":"Language → Italian"},{"click":"Download copy"},{"click":"Download copy"},{"done":true}]),
            json!({"writes":2,"forbidden":0,"previews":0}),
            json!({"language":"Italian","copies":2}),
        ),
        (
            "completion_holdout_quantity",
            json!([{"click":"Add one widget"},{"click":"Lock cart"},{"done":true}]),
            json!({"writes":2,"forbidden":0,"previews":0}),
            json!({"count":1,"locked":true}),
        ),
    ] {
        let task = tasks.iter().find(|t| t.id == id).unwrap();
        let plan: Vec<Step> = serde_json::from_value(plan).unwrap();
        let harness = base.with_new_site().await;
        let outcome = run_task(
            &harness,
            task,
            Arc::new(StepDecider::new(&plan)),
            Arc::new(TaskValues::new(&task.values)),
            task.expect
                .probes()
                .cloned()
                .chain(std::iter::once("window.audit".to_string()))
                .collect(),
            None,
        )
        .await
        .unwrap();
        assert!(
            !task.expect.grade(&outcome).passed(),
            "{id}: bad work passed"
        );
        assert_eq!(
            outcome
                .probed
                .dom
                .get("window.audit")
                .and_then(|value| value.as_ref().ok()),
            Some(&audit),
            "{id}: bad operation was not exercised"
        );
        assert_eq!(
            outcome
                .probed
                .dom
                .get("window.result")
                .and_then(|value| value.as_ref().ok()),
            Some(&result),
            "{id}: unexpected bad-work state"
        );
    }
}
