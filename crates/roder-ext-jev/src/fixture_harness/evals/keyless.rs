//! The keyless tier: every task's scripted plan, on real headless Chrome.
//!
//! Tasks run one at a time in one Chrome, each with its own fixture site so
//! its recorded POSTs are its own. Running them side by side in several
//! Chromes was faster but loaded the machine enough to push the crate's
//! timing-sensitive tests past their margins. `JEV_EVAL_TASKS=id,id` narrows
//! the run. The table goes to stderr (`-- --nocapture` shows it) and the rows
//! to `target/jev-evals/keyless.jsonl`.

use std::sync::Arc;

use super::{
    Row, StepDecider, Task, TaskValues, load_tasks, run_task, table, validate, write_rows,
};
use crate::fixture_harness::Harness;

async fn run_one(base: &Harness, task: &Task) -> Row {
    let harness = base.with_new_site().await;
    let decision = Arc::new(StepDecider::new(&task.script.plan));
    let text = Arc::new(TaskValues::new(&task.values));
    let probes = task
        .expect
        .probes()
        .chain(task.script.expect.probes())
        .cloned()
        .collect::<Vec<_>>();
    match run_task(&harness, task, decision, text, probes).await {
        Ok(outcome) => {
            let mut failures = task.expect.grade(&outcome);
            failures.extend(task.script.expect.grade(&outcome));
            Row::new(task, "keyless", Vec::new(), &outcome, failures)
        }
        Err(error) => Row::errored(task, "keyless", &error),
    }
}

#[tokio::test]
async fn keyless_corpus_passes() {
    let tasks = load_tasks().unwrap();
    validate(&tasks).unwrap();
    let base = harness_or_skip!();
    let mut rows = Vec::new();
    for task in &tasks {
        rows.push(run_one(&base, task).await);
    }
    let path = write_rows("keyless", &rows).unwrap();
    eprintln!("{}rows: {}", table(&tasks, &rows), path.display());
    let failed = rows
        .iter()
        .filter(|row| !row.pass)
        .map(|row| format!("{}: {}", row.task, row.failures.join("; ")))
        .collect::<Vec<_>>();
    assert!(failed.is_empty(), "failed tasks:\n{}", failed.join("\n"));
}
