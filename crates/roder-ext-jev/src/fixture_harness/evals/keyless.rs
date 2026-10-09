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
    FallbackRow, Row, StepDecider, Task, TaskValues, load_tasks, run_task, table, validate,
    write_rows,
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
        Ok(outcome) => {
            let mut graded = task
                .expect
                .grade(&outcome)
                .and(task.script.expect.grade(&outcome));
            let after = task
                .fallback
                .as_ref()
                .map(|_| FallbackRow::grade(task, &outcome, graded.marks()));
            if let Some(after) = &after {
                graded = graded.and(after.missed());
            }
            let mut row = Row::new(task, "keyless", Vec::new(), &outcome, graded);
            row.fallback = after;
            row
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
        .filter(|row| row.marks.false_green)
        .map(|row| format!("{}: {}", row.task, row.failures.join("; ")))
        .collect::<Vec<_>>();
    assert!(
        false_green.is_empty(),
        "false greens (the verdict was right, the page disagrees):\n{}",
        false_green.join("\n")
    );
    let failed = rows
        .iter()
        .filter(|row| !row.passed())
        .map(|row| format!("{}: {}", row.task, row.failures.join("; ")))
        .collect::<Vec<_>>();
    assert!(failed.is_empty(), "failed tasks:\n{}", failed.join("\n"));
}
