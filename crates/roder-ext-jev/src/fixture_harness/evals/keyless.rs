//! The keyless tier: every task's scripted plan, on real headless Chrome.
//!
//! Tasks run one at a time, each with its own fixture site so its recorded
//! POSTs are its own. Running them side by side was faster but loaded the
//! machine enough to push the crate's timing-sensitive tests past their
//! margins. `JEV_EVAL_TASKS=id,id` narrows
//! the run. The table goes to stderr (`-- --nocapture` shows it) and the rows
//! to `target/jev-evals/keyless.jsonl`.

use std::sync::Arc;

use super::{
    FallbackRow, Outcome, Row, StepDecider, Task, TaskValues, load_tasks, run_task, table,
    validate, write_rows,
};
use crate::engine::JevStopCause;
use crate::fallback::model::FallbackModel;
use crate::fixture_harness::Harness;
use crate::fixture_harness::fallback_script::ScriptedFallback;

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
    // A fallback task's scripted fallback; every other task runs Jev alone.
    let fallback = task.fallback.as_ref().map(|fallback| {
        Arc::new(ScriptedFallback::new(fallback.plan.clone())) as Arc<dyn FallbackModel>
    });
    match run_task(&harness, task, decision, text, probes, fallback).await {
        Ok(outcome) => graded_row(task, &outcome),
        Err(error) => Row::errored(task, "keyless", &error),
    }
}

/// The row of a finished keyless run. Its marks are Jev's alone, as the
/// row documents; a task that scripts a fallback also carries what the
/// call came to after it, in `row.fallback`.
pub(super) fn graded_row(task: &Task, outcome: &Outcome) -> Row {
    let graded = task
        .expect
        .grade(outcome)
        .and(task.script.expect.grade(outcome));
    let mut row = Row::new(task, "keyless", Vec::new(), outcome, graded);
    if task.fallback.is_some() {
        row.fallback = Some(FallbackRow::grade(task, outcome, row.marks));
    }
    row
}

/// Every miss that fails a keyless row: Jev's own, then what a scripted
/// fallback left undone (or the fact that it never ran).
pub(super) fn end_to_end_failures(row: &Row) -> Vec<String> {
    let mut failures = row.failures.clone();
    if let Some(after) = &row.fallback {
        failures.extend(after.missed().failures());
    }
    failures
}

/// Whether the keyless gate lets a row through: Jev did what its task asks,
/// and a scripted fallback then finished what the task scripts for it.
pub(super) fn ends_well(row: &Row) -> bool {
    row.passed()
        && row
            .fallback
            .as_ref()
            .is_none_or(|after| after.missed().passed())
}

/// A DONE the page does not back, from Jev or from the scripted fallback
/// after it (its own marks, only when it ran).
pub(super) fn false_green(row: &Row) -> bool {
    row.marks.false_green
        || row
            .fallback
            .as_ref()
            .is_some_and(|after| after.ran && after.marks.false_green)
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
    // The caps on a run going round in circles are insurance: no task of the
    // corpus, which includes runs that keep clicking a counter and runs that
    // wait for a late page, may end on one.
    let capped = rows
        .iter()
        .filter(|row| {
            matches!(
                row.stop_cause,
                Some(JevStopCause::Looped | JevStopCause::Unsettled)
            )
        })
        .map(|row| format!("{}: {:?}", row.task, row.stopped_because))
        .collect::<Vec<_>>();
    assert!(
        capped.is_empty(),
        "tasks ended by a loop cap:\n{}",
        capped.join("\n")
    );
    // The laps account for the run's time: what they miss is the loop's own
    // code and its deliberate waits (the re-reads of an empty first look).
    let (run_ms, laps_ms) = rows
        .iter()
        .filter_map(|row| row.watch.as_ref())
        .fold((0, 0), |(run, laps), watch| {
            (run + watch.run_ms, laps + watch.attributed_ms)
        });
    eprintln!(
        "laps explain {laps_ms} of {run_ms} ms of run time ({:.1}%)",
        laps_ms as f64 * 100.0 / run_ms.max(1) as f64
    );
    assert!(
        laps_ms * 100 >= run_ms * 95,
        "the laps explain only {laps_ms} of {run_ms} ms of run time; see `watch` in {}",
        path.display()
    );
    // A DONE the page does not back is the failure this tier exists to
    // catch before a model does: the scripted corpus has none.
    let false_green = rows
        .iter()
        .filter(|row| false_green(row))
        .map(|row| format!("{}: {}", row.task, end_to_end_failures(row).join("; ")))
        .collect::<Vec<_>>();
    assert!(
        false_green.is_empty(),
        "false greens (the verdict was right, the page disagrees):\n{}",
        false_green.join("\n")
    );
    let failed = rows
        .iter()
        .filter(|row| !ends_well(row))
        .map(|row| format!("{}: {}", row.task, end_to_end_failures(row).join("; ")))
        .collect::<Vec<_>>();
    assert!(failed.is_empty(), "failed tasks:\n{}", failed.join("\n"));
}
